//! Presentation of the simulation's finite fires: the web client's `LocalizedFireEffects`
//! (the web game's `LocalizedFireEffects.ts`).
//!
//! Each burning upperworks zone, main turret's gunhouse or vented compartment is a source: a stream of short-lived,
//! upright flame tongues spread over its roof or hatch, rolling bodies of burning gas that heave up off it and cool
//! into soot, rising embers, and a buoyant column of dark, oily smoke that widens, bends downwind and merges with the
//! ship's other fires into one pall. Her superstructure burns in the open (`naval_sim::fire_zones`: an eighth of her
//! length each, its flames over its broadest roof); a fire below her deck shows its flames only through a shell hole
//! above the water and otherwise only its smoke, from its vent on the deck or the roof over it. A fire being fought
//! mottles its smoke grey with steam, black where the fire breaks through and pale where the hoses reach it, paler
//! as its heat falls; a fire that goes out smoulders for about 45 s, and flooding puts one out in a burst of steam. A
//! fire whose place goes under the sea (she settles, rolls over or plunges) dies there in a burst of steam on the
//! water, and its column, fed no more, is let go: it drifts on and thins from its foot. Camera relevance picks at most
//! 32 sources fleet-wide.

use std::collections::HashMap;

use glam::DVec3;

use crate::camera::CameraView;
use crate::fire_batch::{FireBatch, FireMode, FirePublish};
use crate::instances::{GasPuff, Sprite};
use crate::math::{Rgb, ShipPose, clamp, smoothstep};
use crate::particles::{Align, ParticlePool, PoolKind, SourceId};
use crate::random::Lcg;
use crate::ships::{FireState, FireTrend, SeepPlace, ShipFrame};

const MAX_SOURCES: usize = 32;
/// Fire lights offered to the scene: every fire burning in view, the most relevant first (the web game's two nearest
/// the camera, and the sea's three at any distance, until 2026-10-20), a fire near the lens from up to
/// [`FIRE_LIGHT_POINTS`] points among its flames while there is room (since 2026-10-25). The renderer keeps the
/// brightest at the camera of every light the scene has (`naval_render::PointLights::brightest`, 64).
pub const FIRE_LIGHTS: usize = MAX_SOURCES;
/// Seconds a put-out fire keeps smouldering wisps after its flames die.
const COOLING_SECONDS: f64 = 45.0;
/// Seconds of white steam when flooding puts a fire out.
const STEAM_SECONDS: f64 = 2.2;
const FLAME_CAPACITY: usize = 640;
const SMOKE_CAPACITY: usize = 1600;

/// The heat (kW) a place of scale 1 burning in the open at full intensity gives off, its column's buoyancy: about 30 MW,
/// a gunhouse or a 10 m square of upperworks fully alight (paint, cable, fittings and fuel at 0.25–0.5 MW/m²). It goes
/// as the place's area (its scale squared) and grows with the fire's intensity; a room's fire, which comes out
/// through a hole or a vent, gives half (`column_heat`).
const FIRE_HEAT_KW: f64 = 30_000.0;
/// Smoke seeping out of a burning superstructure's doors, scuttles and windows ([`LocalizedFires::seep`]): the places
/// it may leak from lie this far apart along each of its structures' walls (m), at this height over each deck (m) a
/// deck's height apart (m), up to this far above the roof its flames stand on (m); this share of them leak at the
/// least heat and at full; each puffs this often at full heat (1/s); a ship leaks from this many at most, and only
/// within this range of the camera (m).
const SEEP_SPACING: f64 = 5.0;
const SEEP_SILL: f64 = 1.4;
const SEEP_DECK: f64 = 2.6;
const SEEP_ABOVE: f64 = 3.0;
const SEEP_SHARE: [f64; 2] = [0.12, 0.45];
const SEEP_RATE: f64 = 0.6;
const SEEP_PLACES: usize = 10;
const SEEP_WITHIN: f64 = 1500.0;
/// A fire's light by day ([`PointLight::intensity`], three's candela in the scene's units: the noon sun is about 6 of
/// them, some 16.7 klux each), per kW of its heat: what its flames give off. A fuel fire radiates about a quarter of its
/// heat, and flame at 1,300–1,500 K gives about a tenth of a lumen for each watt of that, so a place of scale 1 fully
/// alight (30 MW) sends out some 750,000 lm, 60,000 cd: 3.6 here. At 10 m that is half a percent of the noon sun, and
/// in the shade of its own smoke a few percent of the sky's light: by day a fire lights nothing about it but its own
/// flames and smoke. (It was 340 a scale before 2026-10-20, the noon sun's own strength at 10 m: under a burning
/// ship's pall her shaded upperworks were lit pink by it.)
const FIRE_LIGHT_PER_KW: f64 = 3.6 / FIRE_HEAT_KW;
/// Metres over which a ship's neighbouring fires' columns merge, adding their buoyancy (`plan_columns`).
const MERGE_REACH: f64 = 40.0;
/// The points a fire's light shines from at most, among its flames (`illuminate`, 2026-10-25): a fire's light came from
/// one point over the middle of its place, so the plating beside flames spread over 20 m took none of it.
const FIRE_LIGHT_POINTS: usize = 4;
/// Flames nearer each other than this (m) shine as one point.
const FIRE_LIGHT_APART: f64 = 4.0;
/// A fire farther from the lens than this (m) shines from one point: its spread does not show there, and the hulls
/// march toward its light only nearer (`naval_render::contact::reach_distance`, some 450 m at the game's lens).
const FIRE_LIGHT_SPREAD: f64 = 500.0;
/// Where a flame's light comes from: this share of its height above its foot, the middle of its luminous body (its
/// core, the brightest, low in it), and how big that body is, as a share of its height (`naval_render::PointLight::
/// radius`: the flame the flipbook draws is some 0.6 of its height across).
const FLAME_LIGHT_HEIGHT: f64 = 0.35;
const FLAME_LIGHT_RADIUS: f64 = 0.3;
/// Seconds of steam where a burning place goes under the sea: the burst of a flooding fire's, as long again, for the sea
/// floods it whole at once.
const DROWNED_STEAM_SECONDS: f64 = 2.0 * STEAM_SECONDS;
/// A column's climb, `rise·e^(−t/τ) + lift` m/s at age t (`FireParticle`), fitted over its first minute to a buoyant
/// plume's (Heskestad): the gas on its axis rises at 1.03·Qc^(1/3)·z^(−1/3) m/s, Qc its convected heat (0.7 of the
/// heat, kW) and z metres above the fire, no faster in the flame than McCaffrey's 1.93·Q^(1/5); the smoke one sees, the
/// plume's mean, at half that. The fit is the same at any heat but for its scale: τ = 8.3 s, and the rise and lift go
/// as Q^(1/4). A 30 MW fire's smoke leaves it at 6.7 m/s and is 50 m up after 10 s and 200 m after a minute.
const COLUMN_RISE_TIME: f64 = 8.3;
const COLUMN_RISE: f64 = 0.30;
const COLUMN_LIFT: f64 = 0.206;
/// The column's widening, which keeps its width at a height what it was before its climb was slowed to the plume's
/// (about a seventh of the height, and its puffs' size at the fire): its growth and diffusion go as Q^(1/4) too.
const COLUMN_GROWTH: f64 = 0.0178;
const COLUMN_GROWTH_DECAY: f64 = 0.055;
const COLUMN_DIFFUSION: f64 = 0.030;
/// The shade of a fought fire's steam, the palest its smoke goes: a light grey, not white, so a fought fire's column
/// reads as the same oil smoke paled by the hoses beside an unfought fire's black (0.78 before 2026-10-17, when the
/// two stood out as two smokes).
const STEAM_SHADE: f64 = 0.45;

/// The top of the sea's boundary layer (m), whose inversion holds a fire's smoke under it however hot the fire: the
/// marine layer's inversion stands some 500 to 1,500 m up.
const INVERSION: f64 = 1_000.0;
/// How long a chained column's puffs may live (s), the most of them: a column's smoke climbs to its top over some
/// minutes and its pall drifts on for minutes more. The chain needs fewer of them the higher and wider it goes
/// (`link_life`), so few live this long.
const PALL_SECONDS: f64 = 420.0;

/// Where a fire's column stops climbing, metres over the fire: Briggs's final rise of a bent-over plume in neutral
/// air, 1.6·F^(1/3)·(3.5·x*)^(2/3)/U, reached x = 3.5·x* downwind (x* = 34·F^(2/5) m, 14·F^(5/8) for F under
/// 55 m⁴/s³), with `column_climb`'s F of the merged `heat` (kW) and U the air going past the fire (`across`, m/s); at
/// most the boundary layer's top ([`INVERSION`]). There its smoke spreads and drifts off downwind as a pall: 184 MW at
/// rest in a 9 m/s wind levels off some 540 m up and three kilometres downwind, at 29 kn into that wind some 210 m up.
fn column_top(heat: f64, across: f64) -> f64 {
    let flux = 0.0194 * heat.max(1.0);
    let reach = if flux < 55.0 {
        14.0 * flux.powf(0.625)
    } else {
        34.0 * flux.powf(0.4)
    };
    (1.6 * flux.cbrt() * (3.5 * reach).powf(2.0 / 3.0) / across.max(0.5)).min(INVERSION)
}

/// The column's climb at its foot, (rise, lift) m/s: its rise off the fire from the fire's `own` heat (kW), its lift
/// higher up from the heat of the columns it has merged with by then (`merged`, its own among them), in air going
/// past the fire at `across` m/s.
///
/// Air crossing a plume bends it over, and then it climbs only as 1.6·F^(1/3)·x^(2/3)/U (Briggs: x metres
/// downwind, U the air's speed past it, F = g·Qc/(cp·T·ρ) its buoyancy flux in m⁴/s³): about the still air's near the
/// fire, less and less above it. Fitted over the first minute, that keeps 0.95 of the rise and 0.56 of the lift times
/// F^(1/12)·U^(−1/3), each at most all of it: a 30 MW fire at 14 kn climbs 0.83 and 0.49 as fast, at 29 kn 0.65 and
/// 0.39 (32 m up after 10 s, against 50 m in still air).
fn column_climb(own: f64, merged: f64, across: f64) -> (f64, f64) {
    // F = 9.81 / (1.005 kJ/kg·K × 293 K × 1.2 kg/m³) × 0.7 Q.
    let bent = |heat: f64| (0.0194 * heat).powf(1.0 / 12.0) * across.max(1e-3).powf(-1.0 / 3.0);
    let (own, merged) = (own.max(1.0), merged.max(own).max(1.0));
    (
        COLUMN_RISE * own.powf(0.25) * (0.95 * bent(own)).min(1.0),
        COLUMN_LIFT * merged.powf(0.25) * (0.56 * bent(merged)).min(1.0),
    )
}

/// A source's own heat for its column (kW, [`FIRE_HEAT_KW`]).
fn column_heat(s: &Source) -> f64 {
    if s.drowned {
        return 0.0;
    }
    FIRE_HEAT_KW
        * s.scale
        * s.scale
        * heat_share(s.burning, s.intensity, s.flame)
        * if s.open { 1.0 } else { 0.5 }
}

/// The share of its full heat a fire gives off, by its intensity while it burns. Once it is out, its heat carries on
/// from where it was and follows its flames (`flame`) down to nothing as they die (a burning fire's intensity is at
/// least 0.08), so it no longer lifts its neighbours' columns.
fn heat_share(burning: bool, intensity: f64, flame: f64) -> f64 {
    if burning {
        0.2 + 0.8 * intensity
    } else {
        (0.2 + 0.8 * flame) * (flame / 0.08).clamp(0.0, 1.0)
    }
}
const EMBER_CAPACITY: usize = 384;
const BODY_CAPACITY: usize = 512;

/// What the presentation remembers about one fire between frames.
#[derive(Clone, Debug, Default)]
struct FireMemory {
    intensity: f64,
    heat: f64,
    heat_rate: f64,
    burned: f64,
    out: f64,
    steam: f64,
    burning: bool,
    flame: f64,
    roll: f64,
    smoke: f64,
    ember: f64,
    wisp: f64,
    /// Trial `flipbook`: puffs its column has been given, which ranks them as links of the chain.
    puffs: u64,
    /// Its place is under the sea: the fire is out whatever the simulation still reads (a hull keel up keeps its
    /// fires' state), and only its steam shows, on the water over it.
    drowned: bool,
    /// It has just gone under: its column is to be let go (`FireBatch::release`).
    release: bool,
}

#[derive(Clone, Debug, Default)]
struct ShipMemories {
    rooms: Vec<Option<FireMemory>>,
    mounts: Vec<Option<FireMemory>>,
    zones: Vec<Option<FireMemory>>,
}

/// Which of a ship's fires a source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Room,
    Mount,
    Zone,
}

/// Which memory a source draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MemoryKey {
    place: Place,
    index: usize,
}

impl MemoryKey {
    /// Its smoke's tag in the batch (`FireParticle::tag`): never 0, which is none.
    fn tag(self) -> u32 {
        let place = match self.place {
            Place::Room => 1,
            Place::Mount => 2,
            Place::Zone => 3,
        };
        (place << 24) | (self.index as u32 & 0x00ff_ffff)
    }
}

#[derive(Clone, Debug)]
struct Source {
    ship: SourceId,
    key: MemoryKey,
    pose: ShipPose,
    velocity: DVec3,
    local: DVec3,
    world: DVec3,
    /// Burning in the open (a gunhouse, her upperworks): flames at any intensity. A hole shows them only once the
    /// fire behind it is fierce.
    open: bool,
    /// It shows flames at all: a room's fire without a hole above the water shows only its smoke.
    flames: bool,
    heading: f64,
    half_x: f64,
    half_z: f64,
    scale: f64,
    burning: bool,
    intensity: f64,
    /// Trial `flipbook`: the intensity its flames are drawn at, which eases to nothing after the fire is out.
    flame: f64,
    fought: bool,
    /// A fought fire: how the party is doing, from −1 (the fire gaining on it) to 1 (the fire dying under the
    /// hoses), by how fast its heat falls (`EffectRules::hoses_gaining`).
    winning: f64,
    growing: bool,
    score: f64,
    distance: f64,
    phase: f64,
    in_view: bool,
    /// Its smoke column's buoyancy (kW): its own heat and its burning neighbours' (`plan_columns`); 0 once it is out
    /// and none of hers burn near it.
    heat: f64,
    /// How fast its column's smoke climbs at its foot, (rise, lift) m/s (`column_climb`), and where it stops climbing,
    /// metres over the fire (`column_top`).
    climb: (f64, f64),
    top: f64,
    /// Its place has gone under the sea: its steam on the water, at `world` (lifted to the surface).
    drowned: bool,
    /// How far its place is lifted onto the swell (burning oil on the water, `ShipEffectsProfile::on_the_water`: the
    /// swell's height under it; 0 for a hull's places, which ride it with her).
    lift: f64,
}

/// A point a fire's light shines from (`LocalizedFires::flame_lights`): where, its share of the fire's light and the
/// size of its flames (m).
#[derive(Clone, Copy, Debug, PartialEq)]
struct FlameLight {
    position: DVec3,
    share: f64,
    radius: f64,
}

/// A flickering point light at a fire, for the layers that light hulls and the sea.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub position: DVec3,
    /// Linear colour.
    pub color: Rgb,
    /// three's point-light intensity (candela), decay 2.
    pub intensity: f64,
    /// Cut-off distance (m).
    pub distance: f64,
    /// The size of the flames it stands for (m, `naval_render::PointLight::radius`; 0 a point).
    pub radius: f64,
    /// Whether the hulls march toward it on screen (`naval_render::lights::LightCaster::Screen`): a fire's after the sun
    /// is down ([`crate::CombatEffects::point_lights`]).
    pub marched: bool,
}

impl From<PointLight> for naval_render::PointLight {
    /// The light as the lit layers take it (`naval_render::lights`): three's point light with decay 2.
    fn from(light: PointLight) -> Self {
        Self {
            position: light.position.as_vec3(),
            color: glam::Vec3::new(
                light.color.r as f32,
                light.color.g as f32,
                light.color.b as f32,
            ),
            intensity: light.intensity as f32,
            distance: light.distance as f32,
            decay: 2.0,
            caster: if light.marched {
                naval_render::lights::LightCaster::Screen
            } else {
                naval_render::lights::LightCaster::None
            },
            radius: light.radius as f32,
            ..Default::default()
        }
    }
}

/// What the fires publish for one frame.
#[derive(Clone, Debug, Default)]
pub struct FiresPublish {
    pub flames: FirePublish,
    pub smoke: FirePublish,
    /// Rolling flame bodies (gas puffs, relit atlas; orange firelight, no soft edge, not an occluder).
    pub bodies: Vec<GasPuff>,
    pub embers: Vec<Sprite>,
    pub lights: Vec<PointLight>,
}

/// A burning source's smoke column, for tools (`effects_probe`): where it rises, the heat it rises on (kW, its
/// neighbours' merged in), how fast its smoke leaves the fire, `rise·e^(−t/τ) + lift` m/s, and where it stops climbing
/// (m over the fire, `column_top`).
#[derive(Clone, Debug, PartialEq)]
pub struct FireColumn {
    pub ship: SourceId,
    pub position: DVec3,
    pub heat: f64,
    pub rise: f64,
    pub lift: f64,
    pub top: f64,
}

/// Counts for diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FireCounts {
    pub sources: usize,
    pub flames: usize,
    pub smoke: usize,
    pub bodies: usize,
    pub embers: usize,
    pub glows: usize,
    pub lights: usize,
}

pub struct LocalizedFires {
    flames: FireBatch,
    smoke: FireBatch,
    bodies: FireBatch,
    /// The bodies' last publish, kept as room (their gas is traded into each frame's publish).
    bodies_room: FirePublish,
    embers: ParticlePool,
    memories: HashMap<SourceId, ShipMemories>,
    sources: Vec<Source>,
    lights: Vec<PointLight>,
    /// Room for a fire's points of light (`flame_lights`).
    light_points: Vec<FlameLight>,
    time: f64,
    random: Lcg,
    lit: usize,
    /// Trial `flipbook`: flames drawn from the baked fire simulation (`flipbook_fires.rs`) in place of the tongues and
    /// the rolling bodies.
    pub(crate) flipbook: bool,
    /// The wind of the last update, for those flames.
    wind: DVec3,
    /// Trial `flipbook`: the smoke columns drawn as chains of a simulated plume (`fire_batch.rs`, `link`) in place of
    /// their puffs' quads.
    pub(crate) column: bool,
    /// Whether the column's heaps can be drawn as billows of the smoke clip ([`crate::flipbook::clip::SMOKE`], from
    /// the fireball's simulation); without it every puff is a link.
    pub(crate) billows: bool,
    /// The swell the hulls ride (`CombatEffects::swell`): a place under it burns no more.
    pub(crate) swell: Option<crate::flotsam::Swell>,
    /// The simulation's rules (`CombatEffects::rules`): how fast a party gains on a fire.
    pub(crate) rules: crate::EffectRules,
}

impl Default for LocalizedFires {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalizedFires {
    pub(crate) fn translate(&mut self, offset: DVec3) {
        self.flames.translate(offset);
        self.smoke.translate(offset);
        self.bodies.translate(offset);
        self.embers.translate(offset);
        for source in &mut self.sources {
            source.world += offset;
            source.pose.x += offset.x;
            source.pose.y += offset.y;
            source.pose.z += offset.z;
        }
    }

    /// Carries the fires into a scene laid on another chart (a theatre's re-lay), each value by its kind: the particles
    /// as their batches and pools carry them, a source's place and her pose's place through their places, her velocity
    /// through the two charts' Jacobians there and her headings through the turn between the grids there. A source's
    /// point on her hull is her own.
    pub(crate) fn relay(&mut self, relay: &dyn naval_render::relay::SceneRelay) {
        self.flames.relay(relay);
        self.smoke.relay(relay);
        self.bodies.relay(relay);
        self.embers.relay(relay);
        for source in &mut self.sources {
            let at = [source.pose.x, source.pose.y, source.pose.z];
            source.velocity = DVec3::from_array(relay.vector(at, source.velocity.to_array()));
            source.heading = relay.heading(at, source.heading);
            source.pose.heading = relay.heading(at, source.pose.heading);
            [source.pose.x, source.pose.y, source.pose.z] = relay.point(at);
            source.world = DVec3::from_array(relay.point(source.world.to_array()));
        }
    }

    /// Carries the fires' smoke columns with the blast waves that passed them this step; their flames stand.
    pub(crate) fn shove(&mut self, shoves: &[crate::shock::Shove]) {
        self.smoke.shove(shoves);
    }

    pub fn new() -> Self {
        Self {
            flames: FireBatch::new(FLAME_CAPACITY, FireMode::Flame),
            smoke: FireBatch::new(SMOKE_CAPACITY, FireMode::Smoke),
            bodies: FireBatch::new(BODY_CAPACITY, FireMode::Gas),
            bodies_room: FirePublish::default(),
            embers: ParticlePool::new(EMBER_CAPACITY, PoolKind::Additive),
            memories: HashMap::new(),
            sources: Vec::new(),
            lights: Vec::new(),
            light_points: Vec::new(),
            time: 0.0,
            random: Lcg::new(1),
            lit: 0,
            flipbook: false,
            wind: DVec3::ZERO,
            column: false,
            billows: false,
            swell: None,
            rules: Default::default(),
        }
    }

    /// Step the fires: age particles, read every hull's fires into memories, keep the most relevant sources and
    /// emit for them. What glows is in the sun's units, as a fire's light is: the host lifts both after dark
    /// (`EffectLighting::lift`).
    pub fn update(
        &mut self,
        dt: f64,
        camera: &CameraView,
        wind: DVec3,
        hidden: Option<&str>,
        ships: &[ShipFrame],
    ) {
        self.update_with_sea(dt, camera, wind, hidden, ships, &[]);
    }

    /// [`Self::update`] with the oil burning on the water besides (`crate::sea_fires`: each burning slick a frame
    /// whose upperworks zones are its cells on the sea).
    pub fn update_with_sea(
        &mut self,
        dt: f64,
        camera: &CameraView,
        wind: DVec3,
        _hidden: Option<&str>,
        ships: &[ShipFrame],
        sea: &[ShipFrame],
    ) {
        self.time += dt;
        self.wind = wind;
        self.smoke.plume = self.column;
        self.flames.advance(dt, wind);
        self.smoke.advance(dt, wind);
        self.bodies.advance(dt, wind);
        self.embers.advance(dt, wind);
        self.collect(dt, camera, ships.iter().chain(sea));
        self.plan_columns();
        if dt > 0.0 {
            self.emit(dt);
            self.seep(dt, camera, ships);
        }
        self.illuminate();
    }

    /// Whether any fire of `ship`'s burns above the water now (one going down with her lights her oil).
    pub fn burning_above_water(&self, ship: &str) -> bool {
        self.memories.get(ship).is_some_and(|m| {
            m.rooms
                .iter()
                .chain(&m.mounts)
                .chain(&m.zones)
                .flatten()
                .any(|f| f.burning && !f.drowned)
        })
    }

    /// Smoke seeping out of a burning superstructure: from places along its structures' walls where a door, a scuttle
    /// or a window would let it out (`ShipEffectsProfile::zone_seeps`), the more of them the harder it burns, a thin dark
    /// puff now and then. Each place's puffs follow from its draw and the clock, so they hold still from frame to frame.
    fn seep(&mut self, dt: f64, camera: &CameraView, ships: &[ShipFrame]) {
        // They give way to the columns, which keep to 0.92 of the smoke's batch (`emit`): none once it is nearly full.
        if !ships
            .iter()
            .any(|ship| ship.zone_fires.iter().any(|f| f.intensity > 0.0))
            || self.smoke.alive() * 20 > SMOKE_CAPACITY * 19
        {
            return;
        }
        let (from, to) = (self.time - dt, self.time);
        for ship in ships {
            let profile = &ship.profile;
            let reach = ship.pose.position().distance(camera.position) - profile.hull_length * 0.5;
            if ship.sunk || reach > SEEP_WITHIN {
                continue;
            }
            let id = Some(&ship.id);
            let mut places = 0;
            for (fire, seeps) in ship.zone_fires.iter().zip(&profile.zone_seeps) {
                if fire.intensity <= 0.0 {
                    continue;
                }
                let share = SEEP_SHARE[0] + (SEEP_SHARE[1] - SEEP_SHARE[0]) * fire.intensity;
                for place in seeps.iter().take_while(|place| place.draw < share) {
                    if places >= SEEP_PLACES {
                        break;
                    }
                    places += 1;
                    // Three more draws of its own from its first: how often it puffs, when, and how large.
                    let [often, phase, large] =
                        [97.0, 389.0, 769.0].map(|k: f64| (place.draw * k).fract());
                    let rate = SEEP_RATE * fire.intensity * (0.5 + often);
                    let puffs = (to * rate + phase).floor() - (from * rate + phase).floor();
                    if puffs < 1.0 {
                        continue;
                    }
                    let outward = ship.pose_at(place.position).rotate(place.outward);
                    let n = (to * rate + phase).floor();
                    let shade = 0.1 + 0.08 * (n * 0.618_034 + place.draw * 7.0).fract();
                    let p = self
                        .smoke
                        .emit(ship.place(place.position + place.outward * 0.3), id);
                    p.tag = 0;
                    p.size = 0.4 + 0.25 * large;
                    p.growth = 0.5;
                    p.growth_decay = 0.4;
                    p.diffusion = 0.06;
                    p.life = 4.0 + 2.0 * often;
                    p.rise = 1.2;
                    p.rise_time = 2.5;
                    p.lift = 0.5;
                    p.drift = ship.velocity * 0.5 + outward * 0.7;
                    p.drag = 0.6;
                    p.shear = 1.0;
                    p.shear_height = 12.0;
                    p.albedo = Rgb::new(shade, shade, shade * 1.02);
                    p.opacity = 0.25 + 0.2 * fire.intensity;
                    p.fade_in = 0.4;
                    p.billow = true;
                }
            }
        }
    }

    fn collect<'a>(
        &mut self,
        dt: f64,
        camera: &CameraView,
        ships: impl IntoIterator<Item = &'a ShipFrame>,
    ) {
        self.sources.clear();
        for ship in ships {
            let burning_anywhere = ship
                .room_fires
                .iter()
                .chain(&ship.mount_fires)
                .chain(&ship.zone_fires)
                .any(|f| f.intensity > 0.0);
            if !self.memories.contains_key(&ship.id) {
                if !burning_anywhere {
                    continue;
                }
                self.memories
                    .insert(ship.id.clone(), ShipMemories::default());
            }
            let pose = ship.pose;
            let profile = &ship.profile;
            // A distant hull whose best possible fire cannot outrank a full list only updates its memories.
            let reach = ((pose.x - camera.position.x).hypot(pose.z - camera.position.z)
                - profile.hull_length * 0.5)
                .max(30.0);
            let outranked = self.sources.len() >= MAX_SOURCES
                && 2.1 / reach <= self.sources[MAX_SOURCES - 1].score;
            let rooms = ship
                .room_fires
                .len()
                .max(self.memories[&ship.id].rooms.len());
            for i in 0..rooms {
                let fire = ship.room_fires.get(i).copied().unwrap_or_default();
                let remembered = self.memories[&ship.id]
                    .rooms
                    .get(i)
                    .is_some_and(Option::is_some);
                if !remembered && fire.intensity <= 0.0 {
                    continue;
                }
                let Some(Some(vent)) = profile.vents.get(i) else {
                    continue;
                };
                let size = vent.size;
                let half = clamp(size[0].min(size[2]) * 0.3, 1.2, 4.5);
                let scale = clamp((size[0] * size[2]).sqrt() / 11.0, 0.6, 1.5);
                // Through a hole above the water: its flames, narrowed to the hole, and its smoke there. Without one:
                // its smoke at the vent.
                let (local, flames, spread) = match ship.openings.get(i).copied().flatten() {
                    Some(hole) => (hole, true, (half.min(1.6), half.min(1.6), scale * 0.8)),
                    None => (vent.position, false, (half, half, scale)),
                };
                let flooded = ship.flooded.get(i).copied().unwrap_or(false);
                let was = was_drowned(&self.memories[&ship.id].rooms, i);
                let drowned =
                    !profile.on_the_water && under_the_sea(&pose, local, self.swell.as_ref(), was);
                let key = MemoryKey {
                    place: Place::Room,
                    index: i,
                };
                let memories = self.memories.get_mut(&ship.id).expect("inserted above");
                let Some(memory) = remember(&mut memories.rooms, i, &fire, dt, flooded, drowned)
                else {
                    continue;
                };
                self.let_go(ship, key, &memory);
                if outranked {
                    continue;
                }
                self.consider(
                    ship,
                    key,
                    &memory,
                    &fire,
                    local,
                    (false, flames),
                    pose.heading,
                    spread,
                    i as f64 * 2.4,
                    camera,
                );
            }
            let mounts = ship
                .mount_fires
                .len()
                .max(self.memories[&ship.id].mounts.len());
            for i in 0..mounts.min(profile.mounts.len()) {
                let fire = ship.mount_fires.get(i).copied().unwrap_or_default();
                let remembered = self.memories[&ship.id]
                    .mounts
                    .get(i)
                    .is_some_and(Option::is_some);
                if !remembered && fire.intensity <= 0.0 {
                    continue;
                }
                let frame = profile.mount_frame(i, &ship.mount_trains);
                let house = profile.mounts[i].gunhouse;
                let local = DVec3::new(frame.x, frame.y + house[2], frame.z);
                let was = was_drowned(&self.memories[&ship.id].mounts, i);
                let drowned =
                    !profile.on_the_water && under_the_sea(&pose, local, self.swell.as_ref(), was);
                let key = MemoryKey {
                    place: Place::Mount,
                    index: i,
                };
                let memories = self.memories.get_mut(&ship.id).expect("inserted above");
                let Some(memory) = remember(&mut memories.mounts, i, &fire, dt, false, drowned)
                else {
                    continue;
                };
                self.let_go(ship, key, &memory);
                if outranked {
                    continue;
                }
                self.consider(
                    ship,
                    key,
                    &memory,
                    &fire,
                    local,
                    (true, true),
                    pose.heading + frame.heading,
                    (
                        house[1] * 0.4,
                        house[0] * 0.4,
                        clamp((house[0] * house[1]).sqrt() / 9.0, 0.45, 1.5),
                    ),
                    1.3 + i as f64 * 2.4,
                    camera,
                );
            }
            // Her upperworks: each zone's fire over its broadest roof.
            let zones = ship
                .zone_fires
                .len()
                .max(self.memories[&ship.id].zones.len());
            for k in 0..zones.min(profile.zones.len()) {
                let fire = ship.zone_fires.get(k).copied().unwrap_or_default();
                let remembered = self.memories[&ship.id]
                    .zones
                    .get(k)
                    .is_some_and(Option::is_some);
                if !remembered && fire.intensity <= 0.0 {
                    continue;
                }
                let Some(Some(zone)) = profile.zones.get(k) else {
                    continue;
                };
                let was = was_drowned(&self.memories[&ship.id].zones, k);
                let drowned = !profile.on_the_water
                    && under_the_sea(&pose, zone.position, self.swell.as_ref(), was);
                let key = MemoryKey {
                    place: Place::Zone,
                    index: k,
                };
                let memories = self.memories.get_mut(&ship.id).expect("inserted above");
                let Some(memory) = remember(&mut memories.zones, k, &fire, dt, false, drowned)
                else {
                    continue;
                };
                self.let_go(ship, key, &memory);
                if outranked {
                    continue;
                }
                self.consider(
                    ship,
                    key,
                    &memory,
                    &fire,
                    zone.position,
                    (true, true),
                    pose.heading,
                    (zone.half[0], zone.half[1], zone.scale),
                    0.7 + k as f64 * 2.4,
                    camera,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn consider(
        &mut self,
        ship: &ShipFrame,
        key: MemoryKey,
        memory: &FireMemory,
        fire: &FireState,
        local: DVec3,
        (open, flames): (bool, bool),
        heading: f64,
        (half_x, half_z, scale): (f64, f64, f64),
        phase: f64,
        camera: &CameraView,
    ) {
        // Once she has broken, where the section holding it is.
        let mut world = ship.place(local);
        if world.y <= 0.0 {
            // Under the sea: nothing but the steam of a fire it drowned, on the water over it.
            if !(memory.drowned && memory.steam > 0.0) {
                return;
            }
            world.y = self
                .swell
                .as_ref()
                .map_or(0.0, |s| s.sea.height(world.x, world.z, s.time));
        }
        // Burning oil rides the swell under it.
        let lift = if ship.profile.on_the_water {
            self.swell
                .as_ref()
                .map_or(0.0, |s| s.sea.height(world.x, world.z, s.time))
        } else {
            0.0
        };
        world.y += lift;
        let distance = (world - camera.position).length();
        // Under the sea it burns no more, whatever the simulation reads of the place.
        let burning = fire.intensity > 0.0 && !memory.drowned;
        let intensity = if burning {
            memory.intensity.max(0.08)
        } else {
            0.0
        };
        let weight = (if burning {
            0.4 + intensity
        } else {
            0.15 + if memory.steam > 0.0 { 0.3 } else { 0.0 }
        }) * scale
            / distance.max(30.0);
        // A full list only takes a fire that can outrank its weakest source, even in plain sight.
        if self.sources.len() >= MAX_SOURCES && weight <= self.sources[MAX_SOURCES - 1].score {
            return;
        }
        let view = camera.to_view(world);
        let depth = -view.z;
        let margin = 40.0 + 20.0 * scale;
        let (lens_x, lens_y) = camera.lens();
        let in_view = depth + margin > 0.0
            && view.x.abs() < depth.max(0.0) / lens_x + margin
            && view.y.abs() < depth.max(0.0) / lens_y + margin * 2.0;
        // A column above an off-screen fire can still be in view; relevance is distance, then sight.
        let score = weight * if in_view { 1.0 } else { 0.35 };
        let fought_of = |fire: &FireState, memory: &FireMemory| {
            fire.suppressed.unwrap_or(memory.heat_rate < -0.004)
        };
        let hoses_gaining = self.rules.hoses_gaining();
        let winning_of = |memory: &FireMemory| (-memory.heat_rate / hoses_gaining).clamp(-1.0, 1.0);
        let growing_of = |fire: &FireState, memory: &FireMemory| match fire.trend {
            Some(trend) => trend == FireTrend::Growing,
            None => memory.heat_rate > 0.004,
        };
        // Several compartments share one authored vent: one source, the strongest fire.
        for s in &mut self.sources {
            if s.ship == ship.id && (s.world - world).length_squared() < 4.0 {
                if score > s.score {
                    s.key = key;
                    s.open = open;
                    s.flames = flames;
                    s.intensity = intensity;
                    s.flame = if burning {
                        intensity
                    } else if memory.drowned {
                        0.0
                    } else {
                        memory.intensity
                    };
                    s.burning = burning;
                    s.score = score;
                    s.fought = fought_of(fire, memory);
                    s.winning = winning_of(memory);
                    s.growing = growing_of(fire, memory);
                }
                return;
            }
        }
        let index = match self.sources.iter().position(|s| s.score < score) {
            Some(index) => index,
            None if self.sources.len() >= MAX_SOURCES => return,
            None => self.sources.len(),
        };
        if self.sources.len() >= MAX_SOURCES {
            self.sources.pop();
        }
        let source = Source {
            ship: ship.id.clone(),
            key,
            pose: ship.pose_at(local),
            velocity: ship.velocity,
            local,
            world,
            open,
            flames,
            heading,
            half_x,
            half_z,
            scale,
            burning,
            intensity,
            flame: if burning {
                intensity
            } else if memory.drowned {
                0.0
            } else {
                memory.intensity
            },
            fought: burning && fought_of(fire, memory),
            winning: winning_of(memory),
            growing: burning && growing_of(fire, memory),
            score,
            distance,
            phase,
            in_view,
            heat: 0.0,
            climb: (0.0, 0.0),
            top: INVERSION,
            drowned: memory.drowned,
            lift,
        };
        let at = index.min(self.sources.len());
        self.sources.insert(at, source);
    }

    /// Each burning source's column: its heat and its neighbours' on the same hull, how fast it climbs in the air
    /// crossing her (the wind at the fire less her way, as her smoke is drawn drifting) and where it stops climbing.
    /// Columns side by side climb apart off their fires and merge as they widen, climbing on as one on their buoyancy
    /// added: all of a fire beside it, a third of one 40 m off.
    fn plan_columns(&mut self) {
        let mut own = [0.0; MAX_SOURCES];
        for (heat, s) in own.iter_mut().zip(&self.sources) {
            *heat = column_heat(s);
        }
        for k in 0..self.sources.len() {
            let s = &self.sources[k];
            let merged: f64 = self
                .sources
                .iter()
                .zip(own)
                .filter(|(o, _)| o.ship == s.ship)
                .map(|(o, heat)| heat * (-(o.world - s.world).length() / MERGE_REACH).exp())
                .sum();
            let air = self.wind * crate::combat::wind_share(s.world.y) - s.velocity;
            let across = air.x.hypot(air.z);
            let climb = column_climb(own[k], merged, across);
            let s = &mut self.sources[k];
            s.heat = merged;
            s.climb = climb;
            s.top = column_top(merged, across);
        }
    }

    fn next(&mut self) -> f64 {
        self.random.next()
    }

    /// A point spread over the source's roof or hatch, in world space.
    fn spread(&mut self, s: &Source, reach: f64) -> DVec3 {
        let ax = (self.next() - 0.5) * 2.0 * s.half_x * reach;
        let az = (self.next() - 0.5) * 2.0 * s.half_z * reach;
        let relative = s.heading - s.pose.heading;
        let (n, c) = relative.sin_cos();
        s.pose.local_to_world(DVec3::new(
            s.local.x + ax * c - az * n,
            s.local.y,
            s.local.z + ax * n + az * c,
        )) + DVec3::Y * s.lift
    }

    fn memory_mut(&mut self, ship: &SourceId, key: MemoryKey) -> &mut FireMemory {
        let memories = self
            .memories
            .get_mut(ship)
            .expect("sources come from memories");
        let list = match key.place {
            Place::Room => &mut memories.rooms,
            Place::Mount => &mut memories.mounts,
            Place::Zone => &mut memories.zones,
        };
        list[key.index]
            .as_mut()
            .expect("sources come from live memories")
    }

    /// A fire whose place has just gone under: its column, fed no more, is let go to thin from its foot.
    fn let_go(&mut self, ship: &ShipFrame, key: MemoryKey, memory: &FireMemory) {
        if !memory.release {
            return;
        }
        self.smoke.release(&ship.id, key.tag());
        self.memory_mut(&ship.id, key).release = false;
    }

    fn emit(&mut self, dt: f64) {
        // Share the smoke batch: expected live puffs never exceed what it holds.
        let demand: f64 = self
            .sources
            .iter()
            .filter(|s| s.burning)
            .map(|s| {
                let i = s.intensity;
                (1.1 + 1.6 * i) * (40.0 + 20.0 * i)
                    / (1.0 + smoothstep(s.distance, 2500.0, 7000.0) * 0.8)
            })
            .sum();
        let budget = (SMOKE_CAPACITY as f64 * 0.92 / demand.max(1.0)).min(1.0);
        // Fire lights its own smoke from below (`glow`, in the sun's units: the night's lift takes it with the flames').
        let sources = std::mem::take(&mut self.sources);
        for s in &sources {
            let i = s.intensity;
            let id = Some(&s.ship);
            let velocity = s.velocity;
            let far = smoothstep(s.distance, 2500.0, 7000.0);
            let mut m = std::mem::take(self.memory_mut(&s.ship, s.key));
            if s.burning && s.in_view {
                // Flame tongues: a steady stream spread over the whole roof or hatch. A hole shows flame only once
                // the fire behind it is fierce, and a room with none shows only smoke.
                let room = if self.flipbook || !s.flames {
                    // Trial `flipbook`: no tongues and no bodies; the simulated flames are drawn at publish.
                    0.0
                } else if s.open {
                    1.0
                } else {
                    smoothstep(i, 0.3, 0.65)
                };
                m.flame += dt
                    * (14.0 + 30.0 * i)
                    * s.scale
                    * room
                    * if s.fought { 0.5 } else { 1.0 }
                    * (1.0 - far * 0.7);
                while m.flame >= 1.0 {
                    m.flame -= 1.0;
                    let at = self.spread(s, if s.open { 0.9 } else { 0.6 });
                    let body = self.next() < 0.35;
                    let variety = 0.55 + self.next() * 0.9;
                    let height = (1.5 + 3.6 * i)
                        * s.scale
                        * if s.open { 1.0 } else { 0.75 }
                        * if s.fought {
                            0.62
                        } else if s.growing {
                            1.12
                        } else {
                            1.0
                        }
                        * variety
                        * if body { 0.6 } else { 1.0 };
                    let aspect = if body {
                        1.0 + self.next() * 0.4
                    } else {
                        0.55 + self.next() * 0.3
                    };
                    let life = (0.45 + self.next() * 0.45) * (height / 4.0).max(0.4).sqrt();
                    let age = -self.next() * dt;
                    let f = self.flames.emit(at, id);
                    f.size = height;
                    f.aspect = aspect;
                    f.life = life;
                    f.rise = 2.4 + 3.0 * i;
                    f.rise_time = 1.5;
                    f.shear = 0.25;
                    f.shear_height = 10.0;
                    f.drift = velocity * 0.94;
                    f.glow = 0.82 + 0.22 * i;
                    f.cool = life * 1.3;
                    f.opacity = 0.96;
                    f.fade_in = 0.06;
                    // Spread births through the frame.
                    f.age = age;
                }
                // Rolling flame: turbulent burning gas heaving up off the fire, cooling into the black column.
                m.roll += dt
                    * (1.8 + 4.0 * i)
                    * s.scale
                    * room
                    * if s.fought { 0.45 } else { 1.0 }
                    * (1.0 - far * 0.6);
                while m.roll >= 1.0 {
                    m.roll -= 1.0;
                    let mut at = self.spread(s, if s.open { 0.7 } else { 0.5 });
                    at.y += 0.6 + self.next() * 0.8;
                    let size = (1.2 + 1.6 * i) * s.scale * (0.7 + self.next() * 0.6);
                    let life = 1.3 + self.next() * 0.6;
                    let rise = (6.0 + 6.0 * i) * (0.8 + self.next() * 0.4);
                    let dx = (self.next() - 0.5) * 2.0;
                    let dz = (self.next() - 0.5) * 2.0;
                    let cooling = (0.6 + self.next() * 0.5) * (0.8 + 0.4 * i);
                    let age = -self.next() * dt;
                    let p = self.bodies.emit(at, id);
                    p.size = size;
                    p.growth = (1.2 + i) * s.scale;
                    p.growth_decay = 0.9;
                    p.diffusion = 0.08;
                    // Short-lived: once cooled it is the column's smoke, which the column's own puffs draw.
                    p.life = life;
                    p.fade_in = 0.08;
                    p.rise = rise;
                    p.rise_time = 2.5;
                    p.lift = 2.5 + i;
                    p.drift = DVec3::new(dx, 0.0, dz) + velocity * 0.9;
                    p.drag = 0.6;
                    p.shear = 1.0;
                    p.shear_height = 20.0;
                    p.heat = (1.6 + 0.4 * i) * if s.fought { 0.7 } else { 1.0 };
                    p.cooling = cooling;
                    p.albedo = Rgb::new(0.05, 0.045, 0.04);
                    p.opacity = 0.95;
                    p.age = age;
                }
                // Embers fly from flames: none from smoke at a vent.
                m.ember += dt
                    * (3.0 + 8.0 * i)
                    * s.scale
                    * if s.fought { 0.4 } else { 1.0 }
                    * (1.0 - far)
                    * if s.flames { 1.0 } else { 0.0 };
                while m.ember >= 1.0 {
                    m.ember -= 1.0;
                    let at = self.spread(s, 0.8);
                    let vx = (self.next() - 0.5) * 4.0;
                    let vy = 5.0 + self.next() * 9.0 * (0.5 + i);
                    let vz = (self.next() - 0.5) * 4.0;
                    let size = 0.1 + self.next() * 0.14;
                    let life = 0.8 + self.next() * 1.6;
                    // Glowing brands of soot and char, a little cooler than the flames they leave (1,380–1,520 K):
                    // dull orange specks by day, bright ones by night. Until 2026-10-24 a cream-orange three times
                    // as bright.
                    let kelvin = 1380.0 + self.next() * 140.0;
                    let e = self.embers.emit(at, id);
                    e.velocity = DVec3::new(vx, vy, vz) + velocity;
                    e.size = size;
                    e.stretch = 2.5;
                    e.align = Align::Velocity;
                    e.life = life;
                    e.gravity = -1.5;
                    e.drag = 0.7;
                    e.wind = 1.0;
                    e.opacity = 0.9;
                    e.color = Rgb::black_body(kelvin);
                }
            }
            if s.burning {
                // The column: buoyant puffs that climb off the fire as a plume does, slowing with height, widen and bend
                // downwind into a pall (`column_climb`). Distant columns use fewer, larger puffs; one ship's fires merge.
                // A fire breathes: its smoke comes off in gusts (`puffing`), swung off its line by the eddies over it
                // (`wander`), so the column is a heap of billows large and small, thick and thin by turns, each standing
                // out of it at its own slant, rather than an even tube.
                let coarse = 1.0 + far * 0.8;
                let pace = s.heat.max(1.0).powf(0.25);
                let gust = puffing(self.time + s.phase * 13.7);
                let swing = wander(self.time + s.phase * 7.3) * (0.3 * (s.climb.0 + s.climb.1));
                // A fought fire: how far the hoses have it now, −1 to 1. They reach it and lose it by turns, and gain
                // on it as its heat falls; the more they have it, the more of its smoke is steam.
                let hold = if s.fought {
                    (0.6 * s.winning + 0.5 * hoses(self.time + s.phase * 11.3)).clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                m.smoke +=
                    dt * (1.1 + 1.6 * i) * if s.fought { 1.2 } else { 1.0 } * budget / coarse;
                while m.smoke >= 1.0 {
                    m.smoke -= 1.0;
                    let steam = s.fought && self.next() < 0.55 + 0.25 * hold;
                    let mut at = self.spread(s, 0.5);
                    at.y += 1.0 + if s.open { 2.0 * i } else { 0.0 };
                    let size = (3.0 + 3.0 * i)
                        * s.scale
                        * (0.7 + self.next() * 0.6)
                        * (1.0 + 0.4 * gust)
                        * coarse.sqrt();
                    // A minute at the plume's pace takes a full fire's smoke about 200 m up.
                    let life = 36.0 + 20.0 * i + self.next() * 8.0;
                    let (rise, lift) = s.climb;
                    let rise = rise * (0.85 + self.next() * 0.3) * if steam { 0.8 } else { 1.0 };
                    let dx = (self.next() - 0.5) * 1.4;
                    let dz = (self.next() - 0.5) * 1.4;
                    let lean = DVec3::new(self.next() - 0.5, 0.0, self.next() - 0.5) * 0.8;
                    // More of a gust's puffs heave up as round heads, thrown off the column's line.
                    let heap = self.next() < 0.3 + 0.25 * gust && self.billows;
                    let thrown = DVec3::new(self.next() - 0.5, 0.0, self.next() - 0.5)
                        * (0.5 * (s.climb.0 + s.climb.1));
                    // Fuel-oil smoke: near black with a brown cast where the sun catches it (its soot's tar takes the blue
                    // most: brown-black in Bunker Hill's and Princeton's photographs); the sky's light and the air between
                    // take most of it back to grey at a distance.
                    let shade = if steam {
                        STEAM_SHADE * (0.85 + self.next() * 0.15)
                    } else {
                        (if s.growing { 0.075 } else { 0.1 })
                            + (1.0 - i) * 0.12
                            + self.next() * 0.04
                    };
                    // A fought column's draws for its links' and billows' shades (`emit` holds the batch).
                    let (streak, tone) = if s.fought && self.column {
                        (self.next(), self.next())
                    } else {
                        (1.0, 0.0)
                    };
                    let age = -self.next() * dt;
                    let p = self.smoke.emit(at, id);
                    p.tag = s.key.tag();
                    p.size = size;
                    p.growth = COLUMN_GROWTH * pace * coarse.sqrt();
                    p.growth_decay = COLUMN_GROWTH_DECAY;
                    p.diffusion = COLUMN_DIFFUSION * pace;
                    p.thin = 0.55;
                    p.life = life;
                    p.rise = rise;
                    p.rise_time = COLUMN_RISE_TIME;
                    p.lift = lift;
                    p.top = s.top;
                    p.drift = DVec3::new(dx, 0.0, dz) + swing + velocity * 0.45;
                    p.drag = 0.35;
                    p.shear = 1.0;
                    p.shear_height = 70.0;
                    p.lean = lean;
                    p.albedo = if steam {
                        Rgb::new(shade, shade * 1.01, shade * 1.03)
                    } else {
                        Rgb::new(shade * 1.18, shade, shade * 0.74)
                    };
                    p.glow = if steam {
                        0.05
                    } else {
                        (0.14 + 0.22 * i) * if s.fought { 0.5 } else { 1.0 }
                    };
                    p.cool = 0.45;
                    p.opacity = if steam { 0.5 } else { 0.7 + 0.15 * i };
                    p.fade_in = 0.5;
                    p.age = age;
                    if self.column {
                        // A link of the chain: it lives while the chain needs it, up the column and on into its pall.
                        p.wake = velocity;
                        p.fade_in = 1.0;
                        // (A puff's seed runs 0 to 100: `golden_seed`.)
                        p.life = PALL_SECONDS * (0.85 + 0.0015 * p.seed);
                        // Thinner between its gusts.
                        p.opacity = (0.9 + 0.1 * i) * (0.85 + 0.15 * gust);
                        if s.fought {
                            // The chain is too few links to mix soot and steam puff by puff (they would stand
                            // black and white by turns), so a fought fire's links are grey with steam, paler while
                            // the hoses have it; a few are its soot breaking through, dark streaks up the column.
                            // Its billows carry the contrast: heads of black oil smoke and bursts of pale steam.
                            let soot = 0.1 + (1.0 - i) * 0.12;
                            let mix = if heap {
                                if steam { 0.9 + 0.1 * tone } else { 0.05 * tone }
                            } else if streak < 0.18 - 0.1 * hold {
                                0.08 + 0.08 * tone
                            } else {
                                0.4 + 0.12 * hold + 0.3 * (p.seed * 0.37).fract()
                            };
                            let pale = soot + (STEAM_SHADE - soot) * mix;
                            p.albedo = Rgb::new(pale, pale, pale * 1.02);
                            p.opacity = if heap && steam {
                                0.6
                            } else {
                                0.75 + 0.1 * (1.0 - mix)
                            };
                            p.glow = (0.14 + 0.22 * i) * 0.5;
                        }
                        if heap {
                            p.billow = true;
                            p.drift += thrown;
                        }
                        let interval =
                            coarse / ((1.1 + 1.6 * i) * if s.fought { 1.2 } else { 1.0 } * budget);
                        p.life = link_life(p, m.puffs, interval, self.wind);
                        m.puffs += 1;
                    }
                }
            } else if m.steam > 0.0 {
                // Put out by flooding: a burst of steam. Gone under the sea: the burst comes off the water over it.
                m.wisp += dt * 7.0 * s.scale;
                while m.wisp >= 1.0 {
                    m.wisp -= 1.0;
                    let mut at = self.spread(s, 0.5);
                    at.y = if s.drowned {
                        s.world.y + 0.6
                    } else {
                        at.y + 0.5
                    };
                    let size = (2.0 + self.next()) * s.scale;
                    let life = 9.0 + self.next() * 4.0;
                    let rise = 10.0 + self.next() * 5.0;
                    let p = self.smoke.emit(at, id);
                    p.tag = s.key.tag();
                    p.size = size;
                    p.growth = 3.2;
                    p.growth_decay = 0.2;
                    p.diffusion = 0.15;
                    p.life = life;
                    p.rise = rise;
                    p.rise_time = 3.0;
                    p.lift = 0.8;
                    p.drift = velocity * 0.4;
                    p.drag = 0.5;
                    p.shear = 1.0;
                    p.shear_height = 30.0;
                    p.albedo = Rgb::new(0.86, 0.88, 0.9);
                    p.opacity = 0.62;
                    p.fade_in = 0.2;
                }
            } else {
                // Put out otherwise: smouldering, thinning wisps.
                let left = 1.0 - m.out / COOLING_SECONDS;
                m.wisp += dt * (0.4 + 1.1 * left) * s.scale;
                while m.wisp >= 1.0 {
                    m.wisp -= 1.0;
                    let mut at = self.spread(s, 0.4);
                    at.y += 0.5;
                    let size = (1.2 + self.next() * 0.8) * s.scale;
                    let life = 12.0 + self.next() * 6.0;
                    let p = self.smoke.emit(at, id);
                    p.tag = s.key.tag();
                    p.size = size;
                    p.growth = 1.3;
                    p.growth_decay = 0.12;
                    p.diffusion = 0.1;
                    p.life = life;
                    p.rise = 3.0 + 2.0 * left;
                    p.rise_time = 5.0;
                    p.lift = 0.6;
                    p.drift = velocity * 0.4;
                    p.drag = 0.4;
                    p.shear = 1.0;
                    p.shear_height = 40.0;
                    let shade = 0.3 + 0.35 * (1.0 - left);
                    p.albedo = Rgb::new(shade, shade, shade * 1.02);
                    p.opacity = 0.18 + 0.3 * left;
                    p.fade_in = 0.6;
                }
            }
            *self.memory_mut(&s.ship, s.key) = m;
        }
        self.sources = sources;
    }

    /// Each burning fire in view lights the scene (its ship, the ships about her, the sea) at the light its flames give
    /// off. Until 2026-10-20 it also glowed: a soft halo sprite about its flames and a flickering pool laid on the sea
    /// under it, which stood in for the glare about a bright light and for its light on the water; lifted with the night
    /// as the flames are, they drew a fire as a lit fog, and the bloom (the eye's own glare) and the sea's point lights
    /// give both.
    /// Every fire burning in view as light on the scene, the most relevant first, at most [`FIRE_LIGHTS`] lights in
    /// all: [`FIRE_LIGHT_PER_KW`] of its heat, flickering, where its flames stand (`flame_lights`). A fire near the
    /// lens whose flames stand apart shines from up to [`FIRE_LIGHT_POINTS`] points among them, its light split by
    /// their flames' size; when the list is full the last fires keep one point each.
    fn illuminate(&mut self) {
        self.lights.clear();
        let mut points = std::mem::take(&mut self.light_points);
        let burning: Vec<usize> = (0..self.sources.len())
            .filter(|&i| {
                let s = &self.sources[i];
                // Smoke from a vent casts no firelight: its flames are below her deck.
                s.burning && s.in_view && s.flames
            })
            .collect();
        for (n, &i) in burning.iter().enumerate() {
            if self.lights.len() >= FIRE_LIGHTS {
                break;
            }
            let s = &self.sources[i];
            // Room for this fire's points that leaves one for each fire after it.
            let left = burning.len() - n - 1;
            let room = (FIRE_LIGHTS - self.lights.len())
                .saturating_sub(left)
                .clamp(1, FIRE_LIGHT_POINTS);
            let room = if s.distance > FIRE_LIGHT_SPREAD {
                1
            } else {
                room
            };
            self.flame_lights(s, room, &mut points);
            let fought = if s.fought { 0.6 } else { 1.0 };
            let power = FIRE_LIGHT_PER_KW * column_heat(s) * fought;
            for (k, point) in points.iter().enumerate() {
                // Each point flickers on its own: the light moves among the flames.
                let t = self.time + s.phase + k as f64 * 2.3;
                let flicker = 0.8
                    + 0.1 * (t * 9.1).sin()
                    + 0.06 * (t * 23.7 + 1.3).sin()
                    + 0.04 * (t * 41.9 + 2.1).sin();
                self.lights.push(PointLight {
                    position: point.position,
                    color: Rgb::hex(0xff8a3c),
                    intensity: power * point.share * flicker,
                    distance: 0.0,
                    radius: point.radius,
                    marched: false,
                });
            }
        }
        self.lit = burning.len();
        self.light_points = points;
    }

    /// Where `s`'s light comes from, at most `room` points, their shares of its light adding to 1 (written over `out`):
    /// its flames as the flipbook draws them, each at the middle of its body ([`FLAME_LIGHT_HEIGHT`], leaning as it
    /// leans) and as big ([`FLAME_LIGHT_RADIUS`]), with a share as its flame's size; the nearest two of them taken as
    /// one (at their weighted middle) while there are more than `room` or two stand within [`FIRE_LIGHT_APART`].
    /// Without the flipbook's flames, or a hatch's fire too low to show any, one point over the middle of its place as
    /// before.
    fn flame_lights(&self, s: &Source, room: usize, out: &mut Vec<FlameLight>) {
        out.clear();
        if self.flipbook {
            let mut sprites = Vec::new();
            let air = crate::flipbook_fires::Air {
                time: self.time,
                wind: self.wind,
            };
            crate::flipbook_fires::flames(&Self::flame_of(s), &air, &mut sprites);
            for (_, sprite) in &sprites {
                let tile = 2.0 * f64::from(sprite.sphere[3]);
                let height = tile * crate::flipbook_fires::FLAME_SHARE;
                let foot = DVec3::new(
                    f64::from(sprite.sphere[0]),
                    f64::from(sprite.sphere[1]) + tile * crate::flipbook_fires::SINK,
                    f64::from(sprite.sphere[2]),
                );
                let up = height * FLAME_LIGHT_HEIGHT;
                let lean = DVec3::new(f64::from(sprite.quad[0]), 1.0, f64::from(sprite.quad[2]));
                out.push(FlameLight {
                    position: foot + lean * up,
                    share: height * height * f64::from(sprite.extra[0]),
                    radius: height * FLAME_LIGHT_RADIUS,
                });
            }
        }
        let total: f64 = out.iter().map(|point| point.share).sum();
        if out.is_empty() || total <= 0.0 {
            out.clear();
            let mut position = s.world;
            position.y += 2.5 + 2.5 * s.scale * s.intensity;
            out.push(FlameLight {
                position,
                share: 1.0,
                radius: (2.3 + 5.5 * s.intensity) * s.scale * FLAME_LIGHT_RADIUS,
            });
            return;
        }
        for point in out.iter_mut() {
            point.share /= total;
        }
        while out.len() > 1 {
            let mut nearest = (f64::INFINITY, 0, 0);
            for a in 0..out.len() {
                for b in a + 1..out.len() {
                    let apart = out[a].position.distance(out[b].position);
                    if apart < nearest.0 {
                        nearest = (apart, a, b);
                    }
                }
            }
            let (apart, a, b) = nearest;
            if out.len() <= room && apart >= FIRE_LIGHT_APART {
                break;
            }
            let (first, second) = (out[a], out.swap_remove(b));
            let share = first.share + second.share;
            out[a] = FlameLight {
                position: (first.position * first.share + second.position * second.share) / share,
                share,
                // Their flames' areas added, and the reach between them.
                radius: first.radius.hypot(second.radius).max(0.5 * apart),
            };
        }
    }

    /// The fire lights `update` chose this frame: every fire burning in view, the most relevant first (the lens's own
    /// hull's included: her fire lights the sea about her), in the scene's units by day.
    pub fn lights(&self) -> &[PointLight] {
        &self.lights
    }

    pub fn publish(&mut self, camera: &CameraView, hidden: Option<&str>) -> FiresPublish {
        let mut publish = FiresPublish::default();
        self.publish_into(camera, hidden, &mut publish);
        publish
    }

    /// [`LocalizedFires::publish`] written over `out`, whose lists keep their room.
    pub fn publish_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        out: &mut FiresPublish,
    ) {
        self.flames.publish_into(camera, hidden, &mut out.flames);
        self.smoke.publish_into(camera, hidden, &mut out.smoke);
        // The bodies publish as gas: theirs is traded into `out`, last frame's list kept as the room.
        self.bodies
            .publish_into(camera, hidden, &mut self.bodies_room);
        std::mem::swap(&mut out.bodies, &mut self.bodies_room.gas);
        if self.flipbook {
            self.flipbook_flames(camera, hidden, &mut out.bodies);
        }
        out.embers.clear();
        self.embers
            .publish_sprites_into(camera, hidden, &mut out.embers);
        out.lights.clone_from(&self.lights);
    }

    /// The smoke columns' batch, for the volume trial (`volume.rs`), which draws the nearest ships' columns itself.
    pub(crate) fn smoke_batch_mut(&mut self) -> &mut FireBatch {
        &mut self.smoke
    }

    /// `s`'s flames as the flipbook draws them (`flipbook_fires.rs`), for its sprites and its light.
    fn flame_of(s: &Source) -> crate::flipbook_fires::Fire<'_> {
        crate::flipbook_fires::Fire {
            ship: &s.ship,
            // Rooms and mounts keep the keys they had before her upperworks burned, so their flames keep their
            // phases.
            key: s.key.index as u64 * 2
                + match s.key.place {
                    Place::Room => 0,
                    Place::Mount => 1,
                    Place::Zone => 1 << 40,
                },
            pose: ShipPose {
                y: s.pose.y + s.lift,
                ..s.pose
            },
            velocity: s.velocity,
            local: s.local,
            heading: s.heading,
            half_x: s.half_x,
            half_z: s.half_z,
            scale: s.scale,
            mount: s.open,
            intensity: s.flame,
            fought: s.fought,
            growing: s.growing,
            distance: s.distance,
        }
    }

    /// Trial `flipbook`: every fire in view as sprites of the simulated fire, back to front, after the bodies.
    fn flipbook_flames(&self, camera: &CameraView, hidden: Option<&str>, out: &mut Vec<GasPuff>) {
        let fires = self
            .sources
            .iter()
            .filter(|s| s.in_view && s.flames && !s.drowned && Some(&*s.ship) != hidden)
            .map(Self::flame_of);
        let air = crate::flipbook_fires::Air {
            time: self.time,
            wind: self.wind,
        };
        crate::flipbook_fires::publish(fires, &air, camera, out);
    }

    pub fn set_density(&mut self, density: f64) {
        self.flames.density = density;
        self.smoke.density = density;
        self.bodies.density = density;
        self.embers.density = density;
    }

    /// Every burning source's column as the last update planned it.
    pub fn columns(&self) -> Vec<FireColumn> {
        self.sources
            .iter()
            .filter(|s| s.burning)
            .map(|s| FireColumn {
                ship: s.ship.clone(),
                position: s.world,
                heat: s.heat,
                rise: s.climb.0,
                lift: s.climb.1,
                top: s.top,
            })
            .collect()
    }

    pub fn counts(&self) -> FireCounts {
        FireCounts {
            sources: self.sources.len(),
            flames: self.flames.alive(),
            smoke: self.smoke.alive(),
            bodies: self.bodies.alive(),
            embers: self.embers.alive(),
            glows: self.lit,
            lights: self.lights.len(),
        }
    }

    /// Trial (shadows): the smoke columns and flame bodies as the shadow map's casters (`shadow.rs`).
    pub(crate) fn shadow_blobs(&self, hidden: Option<&str>, out: &mut Vec<crate::shadow::Blob>) {
        crate::shadow::fire_blobs(&self.smoke, hidden, out);
        crate::shadow::fire_blobs(&self.bodies, hidden, out);
    }

    pub fn reset(&mut self) {
        self.flames.reset();
        self.smoke.reset();
        self.bodies.reset();
        self.embers.reset();
        self.memories.clear();
        self.sources.clear();
        self.lights.clear();
        self.time = 0.0;
        self.random = Lcg::new(1);
        self.lit = 0;
    }
}

/// Trial `flipbook`: how long a puff of a smoke column lives as a link of its chain. Links leave the fire closer
/// together than their length and grow as they slow, so the chain needs fewer of them the higher it goes: every
/// other puff goes once the links are twice as close as they need be, every other of the rest at four times, and so
/// on (`count` is the puff's number in its column; `interval` the seconds between puffs). What is left stands about a
/// puff's radius and a half apart all the way up and along its pall, where the game's column draws every puff to the
/// top: a handful of the widest links, minutes old, make the pall kilometres downwind.
fn link_life(p: &crate::fire_batch::FireParticle, count: u64, interval: f64, wind: DVec3) -> f64 {
    /// Links wanted along a column, in puff radii between them: half a link's length, at which the links' smoke
    /// (which thins to nothing at each end) adds up to an even body. The ranks leave them between that and half of it.
    const APART: f64 = 1.6;
    let rank = count.trailing_zeros().min(10);
    let mut probe = p.clone();
    let mut time = 0.0;
    while time < p.life {
        // Finely while the chain thins fast over the fire, coarsely along the pall.
        time += if time < 30.0 { 0.5 } else { 2.0 };
        probe.age = time;
        let (climb, rising) = p.climb_at(time);
        let across =
            (p.drift * (-p.drag * time).exp() + wind * p.carried_at(climb) - p.wake).length();
        let apart = rising.hypot(across) * interval;
        // Faded half away by the time its neighbours stand close enough without it.
        if apart * f64::from(1u32 << (rank + 1)) < APART * probe.extent() {
            return (time / 0.85).clamp(4.0, p.life);
        }
    }
    p.life
}

/// How hard a fire is puffing out its smoke at `t` s, −1 to 1: swells of 3 to 9 s, so its column's billows come
/// large and small, thick and thin by turns.
fn puffing(t: f64) -> f64 {
    (0.55 * (t * 1.13).sin() + 0.3 * (t * 2.31 + 1.7).sin() + 0.25 * (t * 0.71 + 4.1).sin())
        .clamp(-1.0, 1.0)
}

/// How far a party's hoses have its fire at `t` s, −1 to 1: they reach it and lose it by turns over 13 to 27 s.
fn hoses(t: f64) -> f64 {
    0.6 * (t * 0.47).sin() + 0.4 * (t * 0.23 + 2.3).sin()
}

/// Which way the eddies over a fire swing its smoke off its line at `t` s: a horizontal vector of at most about
/// unit length, turning over 7 to 22 s.
fn wander(t: f64) -> DVec3 {
    DVec3::new(
        (t * 0.37).sin() + 0.5 * (t * 0.83 + 2.0).sin(),
        0.0,
        (t * 0.29 + 1.3).sin() + 0.5 * (t * 0.71 + 0.4).sin(),
    ) / 1.5
}

/// How far past the sea's surface (m) a place must go to be under it, and to be out of it again: a hole by her water
/// line dipping into a wave as she rolls does not put her fire out and light it again with every roll.
const DROWN_MARGIN: f64 = 0.5;

/// Whether a place of hers (`local`, in her frame) lies under the sea: the swell she rides where it is (`swell`; the
/// flat sea without one), by [`DROWN_MARGIN`] going under and coming out (`was` it under last frame).
fn under_the_sea(
    pose: &ShipPose,
    local: DVec3,
    swell: Option<&crate::flotsam::Swell>,
    was: bool,
) -> bool {
    let at = pose.local_to_world(local);
    let depth = swell.map_or(0.0, |s| s.sea.height(at.x, at.z, s.time)) - at.y;
    if was {
        depth > -DROWN_MARGIN
    } else {
        depth > DROWN_MARGIN
    }
}

/// Whether the memory at `index` of `list` was under the sea last frame.
fn was_drowned(list: &[Option<FireMemory>], index: usize) -> bool {
    list.get(index)
        .and_then(Option::as_ref)
        .is_some_and(|m| m.drowned)
}

/// Update one fire's memory; `None` once nothing of it remains to draw. Returns a copy for ranking. A place under the
/// sea (`drowned`) burns no more: one that burned there goes out in a burst of steam, and its column is let go.
fn remember(
    list: &mut Vec<Option<FireMemory>>,
    index: usize,
    fire: &FireState,
    dt: f64,
    flooded: bool,
    drowned: bool,
) -> Option<FireMemory> {
    if list.len() <= index {
        list.resize(index + 1, None);
    }
    // Under the sea nothing burns, whatever the simulation reads of the place.
    let burning = fire.intensity > 0.0 && !drowned;
    let intensity = if drowned { 0.0 } else { fire.intensity };
    // Where the view carries no heat (an observed hull, a place not burning), intensity stands in for it.
    let heat = fire.heat.unwrap_or(fire.intensity);
    if list[index].is_none() {
        if !burning {
            return None;
        }
        list[index] = Some(FireMemory {
            heat,
            ..FireMemory::default()
        });
    }
    let m = list[index].as_mut().expect("set above");
    if dt > 0.0 {
        m.heat_rate += ((heat - m.heat) / dt - m.heat_rate) * (dt / 1.5).min(1.0);
        m.heat = heat;
        // Ease the displayed intensity, so a fire grows into view and dies down instead of popping.
        let ease = if intensity > m.intensity { 1.6 } else { 0.9 };
        m.intensity += (intensity - m.intensity) * (dt / ease).min(1.0);
        if burning {
            m.burned += dt;
            m.out = 0.0;
        } else if m.burned > 0.0 {
            m.out += dt;
        }
        m.steam = if burning {
            0.0
        } else {
            (m.steam - dt).max(0.0)
        };
        if m.burning && !burning && drowned {
            m.steam = DROWNED_STEAM_SECONDS;
        } else if m.burning && !burning && flooded {
            m.steam = STEAM_SECONDS;
        }
        // Just gone under: its column is let go, burning or smouldering.
        m.release |= drowned && !m.drowned;
    }
    m.drowned = drowned;
    m.burning = burning;
    // A fire gone under keeps nothing but its steam: no smoulder.
    if !burning && m.steam <= 0.0 && (drowned || m.burned <= 0.0 || m.out > COOLING_SECONDS) {
        list[index] = None;
        return None;
    }
    Some(m.clone())
}

/// The places smoke may seep out of a burning upperworks zone (`ShipEffectsProfile::zone_seeps`), its structures'
/// `boxes` (ship-local), its flames standing on the roof at `seat` (m): along each wall every [`SEEP_SPACING`], a
/// sill's height over each deck up to a little above that roof, not where another of its structures stands against
/// the wall; each with its draw (hashed from its box, wall, deck and place), the least first.
pub fn seep_places(boxes: &[[DVec3; 2]], seat: f64) -> Vec<SeepPlace> {
    let hash = |words: [u32; 4]| {
        let mut h = words
            .iter()
            .fold(0x811c_9dc5u32, |h, &w| (h ^ w).wrapping_mul(0x0100_0193));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        f64::from(h) / 4_294_967_296.0
    };
    let mut places = Vec::new();
    for (b, &[low, high]) in boxes.iter().enumerate() {
        // Its four walls: along each, its outward normal (ship-local).
        let walls = [
            (
                DVec3::new(high.x, 0.0, low.z),
                DVec3::new(high.x, 0.0, high.z),
                DVec3::X,
            ),
            (
                DVec3::new(low.x, 0.0, low.z),
                DVec3::new(low.x, 0.0, high.z),
                DVec3::NEG_X,
            ),
            (
                DVec3::new(low.x, 0.0, low.z),
                DVec3::new(high.x, 0.0, low.z),
                DVec3::NEG_Z,
            ),
            (
                DVec3::new(low.x, 0.0, high.z),
                DVec3::new(high.x, 0.0, high.z),
                DVec3::Z,
            ),
        ];
        for (w, (start, end, outward)) in walls.into_iter().enumerate() {
            let count = (start.distance(end) / SEEP_SPACING).floor().max(1.0) as u32;
            let (mut y, mut deck) = (low.y + SEEP_SILL, 0u32);
            while y < high.y - 0.3 && y < seat + SEEP_ABOVE {
                for i in 0..count {
                    let mut position = start.lerp(end, (f64::from(i) + 0.5) / f64::from(count));
                    position.y = y;
                    let p = position + outward * 0.4;
                    if boxes
                        .iter()
                        .any(|[l, h]| p.cmpgt(*l).all() && p.cmplt(*h).all())
                    {
                        continue;
                    }
                    let draw = hash([b as u32, w as u32, deck, i]);
                    places.push(SeepPlace {
                        position,
                        outward,
                        draw,
                    });
                }
                y += SEEP_DECK;
                deck += 1;
            }
        }
    }
    places.sort_by(|a, b| a.draw.total_cmp(&b.draw));
    places
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ships::{FireVent, ShipEffectsProfile, ZoneVent};
    use glam::DMat4;
    use std::sync::Arc;

    /// A deckhouse with a second standing against its side, flames on its roof at 9 m: its seep places lie on its
    /// walls a sill over each deck and no higher than a little above that roof, none opening into the other, least
    /// draw first.
    #[test]
    fn seeps_leave_by_open_walls_below_the_roof() {
        let boxes = [
            [DVec3::new(-5.0, 3.0, -10.0), DVec3::new(5.0, 9.0, 10.0)],
            [DVec3::new(5.0, 3.0, -4.0), DVec3::new(9.0, 20.0, 4.0)],
        ];
        let places = seep_places(&boxes, 9.0);
        assert!(!places.is_empty());
        assert!(places.windows(2).all(|pair| pair[0].draw <= pair[1].draw));
        for place in &places {
            assert!(place.position.y < 9.0 + SEEP_ABOVE && place.position.y >= 3.0 + SEEP_SILL);
            let out = place.position + place.outward * 0.4;
            assert!(
                !boxes
                    .iter()
                    .any(|[l, h]| out.cmpgt(*l).all() && out.cmplt(*h).all())
            );
        }
        // The tall house's walls leak only up to a little above the roof the flames stand on.
        assert!(places.iter().any(|p| p.position.x > 8.0));
    }

    /// A room burning hard at night 300 m off, with a shell hole above the water or without one: its sources and
    /// what the fires emitted.
    fn room_fire(hole: bool) -> (Vec<Source>, FireCounts) {
        let profile = Arc::new(ShipEffectsProfile {
            hull_length: 180.0,
            hull_beam: 20.0,
            vents: vec![Some(FireVent {
                position: DVec3::new(0.0, 8.0, 0.0),
                size: [12.0, 6.0, 16.0],
                capacity_m3: 900.0,
            })],
            ..Default::default()
        });
        let mut ship = ShipFrame::new("ship", profile, ShipPose::default());
        ship.room_fires = vec![FireState {
            intensity: 1.0,
            ..Default::default()
        }];
        if hole {
            ship.openings = vec![Some(DVec3::new(10.0, 4.0, 2.0))];
        }
        let eye = DVec3::new(0.0, 20.0, 300.0);
        let camera = CameraView {
            position: eye,
            view: glam::dcamera::rh::view::look_at_mat4(eye, DVec3::ZERO, DVec3::Y),
            projection: glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                45f64.to_radians(),
                16.0 / 9.0,
                0.5,
            ),
            near: 0.5,
            fov_y_degrees: 45.0,
            height: 720.0,
            curvature: 0.0,
        };
        let mut fires = LocalizedFires::new();
        for _ in 0..120 {
            fires.update(
                1.0 / 30.0,
                &camera,
                DVec3::ZERO,
                None,
                std::slice::from_ref(&ship),
            );
        }
        (fires.sources.clone(), fires.counts())
    }

    #[test]
    fn a_room_burning_shows_flames_through_a_shell_hole_and_only_smoke_without_one() {
        let (sources, counts) = room_fire(false);
        assert_eq!(sources.len(), 1);
        assert!(!sources[0].flames);
        assert!(counts.smoke > 0, "its smoke rises from the vent");
        assert_eq!((counts.flames, counts.embers, counts.lights), (0, 0, 0));
        let (sources, counts) = room_fire(true);
        assert!(sources[0].flames && !sources[0].open);
        assert_eq!(sources[0].local, DVec3::new(10.0, 4.0, 2.0), "at the hole");
        assert!(counts.flames > 0 && counts.lights > 0);
    }

    /// Four burning ships straight ahead of a camera, 0.8, 3.3, 6.3 and 9.3 km off, at night.
    fn burning_line() -> (LocalizedFires, CameraView) {
        let profile = Arc::new(ShipEffectsProfile {
            hull_length: 180.0,
            hull_beam: 20.0,
            zones: vec![Some(ZoneVent {
                position: DVec3::new(0.0, 8.0, 0.0),
                half: [5.0, 6.7],
                scale: 1.3,
            })],
            ..Default::default()
        });
        let ships: Vec<ShipFrame> = [500.0, 3000.0, 6000.0, 9000.0]
            .iter()
            .enumerate()
            .map(|(i, distance)| {
                let pose = ShipPose {
                    x: 0.0,
                    y: 0.0,
                    z: -distance,
                    heading: 0.0,
                    pitch: 0.0,
                    roll: 0.0,
                };
                let mut ship = ShipFrame::new(&format!("ship-{i}"), profile.clone(), pose);
                ship.zone_fires = vec![FireState {
                    intensity: 1.0,
                    ..Default::default()
                }];
                ship
            })
            .collect();
        let eye = DVec3::new(0.0, 20.0, 300.0);
        let camera = CameraView {
            position: eye,
            view: glam::dcamera::rh::view::look_at_mat4(
                eye,
                DVec3::new(0.0, 0.0, -9000.0),
                DVec3::Y,
            ),
            projection: DMat4::IDENTITY,
            near: 0.5,
            fov_y_degrees: 90.0,
            height: 1000.0,
            curvature: 0.0,
        };
        let mut fires = LocalizedFires::new();
        for _ in 0..30 {
            fires.update(1.0 / 60.0, &camera, DVec3::ZERO, None, &ships);
        }
        (fires, camera)
    }

    /// One ship burning at full intensity 130 m off the lens's beam for 70 s, at `speed` m/s: the fires and what
    /// they publish at the end.
    fn column(chain: bool, speed: f64) -> (LocalizedFires, FiresPublish) {
        burning(chain, speed, |_| FireState {
            intensity: 1.0,
            ..Default::default()
        })
    }

    /// One ship's fire, as `fire` reports it `t` s in, 130 m off the lens's beam for 70 s at `speed` m/s: the fires
    /// and what they publish at the end.
    fn burning(
        chain: bool,
        speed: f64,
        fire: impl Fn(f64) -> FireState,
    ) -> (LocalizedFires, FiresPublish) {
        burning_in(chain, speed, 70, DVec3::new(2.4, 0.0, 0.9), fire)
    }

    /// One ship's fire, as `fire` reports it `t` s in, 130 m off the lens's beam for `seconds` at `speed` m/s in a
    /// `wind` (the 10 m wind): the fires and what they publish at the end.
    fn burning_in(
        chain: bool,
        speed: f64,
        seconds: u32,
        wind: DVec3,
        fire: impl Fn(f64) -> FireState,
    ) -> (LocalizedFires, FiresPublish) {
        let profile = Arc::new(ShipEffectsProfile {
            hull_length: 180.0,
            hull_beam: 20.0,
            vents: vec![Some(FireVent {
                position: DVec3::new(0.0, 8.0, 0.0),
                size: [12.0, 6.0, 16.0],
                capacity_m3: 900.0,
            })],
            ..Default::default()
        });
        let mut fires = LocalizedFires::new();
        fires.flipbook = chain;
        fires.column = chain;
        fires.billows = chain;
        let mut camera = None;
        let mut x = 0.0;
        for frame in 0..seconds * 60 {
            x += speed / 60.0;
            let pose = ShipPose {
                x,
                y: 0.0,
                z: 0.0,
                heading: std::f64::consts::FRAC_PI_2,
                pitch: 0.0,
                roll: 0.0,
            };
            let mut ship = ShipFrame::new("ship", profile.clone(), pose);
            ship.velocity = DVec3::new(speed, 0.0, 0.0);
            ship.room_fires = vec![fire(f64::from(frame) / 60.0)];
            let eye = DVec3::new(x, 26.0, 130.0);
            let view = CameraView {
                position: eye,
                view: glam::dcamera::rh::view::look_at_mat4(
                    eye,
                    DVec3::new(x, 60.0, 0.0),
                    DVec3::Y,
                ),
                projection: glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                    45f64.to_radians(),
                    16.0 / 9.0,
                    0.5,
                ),
                near: 0.5,
                fov_y_degrees: 45.0,
                height: 720.0,
                curvature: 0.0,
            };
            fires.update(1.0 / 60.0, &view, wind, None, &[ship]);
            camera = Some(view);
        }
        let publish = fires.publish(&camera.expect("stepped"), None);
        (fires, publish)
    }

    /// Metres a plume's smoke has climbed `t` s after leaving a fire of `heat` kW 2 m up in its flame, integrated
    /// from the plume's own law (half its axis's speed: Heskestad's 1.03·Qc^(1/3)·z^(−1/3), Qc = 0.7 Q, at most
    /// McCaffrey's 1.93·Q^(1/5) in the flame).
    fn plume_height(heat: f64, t: f64) -> f64 {
        let (mut z, mut time, dt) = (2.0f64, 0.0, 0.01);
        while time < t {
            let axis = (1.03 * (0.7 * heat).cbrt() * z.powf(-1.0 / 3.0)).min(1.93 * heat.powf(0.2));
            z += 0.5 * axis * dt;
            time += dt;
        }
        z - 2.0
    }

    /// The analytic climb of a column's smoke with `climb` (rise, lift) after `t` s.
    fn climbed((rise, lift): (f64, f64), t: f64) -> f64 {
        rise * COLUMN_RISE_TIME * (1.0 - (-t / COLUMN_RISE_TIME).exp()) + lift * t
    }

    #[test]
    fn a_column_climbs_as_a_fires_plume_does() {
        for heat in [5_000.0, 30_000.0, 120_000.0] {
            // In still air: within a tenth of the plume over its first minute, and no faster off the fire than the
            // smoke of a flame that size rises.
            let still = column_climb(heat, heat, 0.0);
            for t in [2.0, 5.0, 10.0, 20.0, 40.0, 60.0] {
                let (game, plume) = (climbed(still, t), plume_height(heat, t));
                assert!(
                    (game / plume - 1.0).abs() < 0.12,
                    "{heat} kW at {t} s: {game:.0} m against the plume's {plume:.0} m"
                );
            }
            assert!(still.0 + still.1 < 0.5 * 1.93 * heat.powf(0.2) * 1.05);
            // Bent over by the air going past her at 14 kn: lower, as Briggs's 1.6·F^(1/3)·x^(2/3)/U has it after
            // the first seconds, within a fifth.
            let across = 7.2;
            let bent = column_climb(heat, heat, across);
            let flux = 9.81 / (1.005 * 293.0 * 1.2) * 0.7 * heat;
            for t in [10.0, 30.0, 60.0] {
                let (game, briggs) = (
                    climbed(bent, t),
                    1.6 * flux.cbrt() * (across * t).powf(2.0 / 3.0) / across,
                );
                assert!(game < climbed(still, t));
                assert!(
                    (game / briggs - 1.0).abs() < 0.2,
                    "{heat} kW at {t} s in {across} m/s: {game:.0} m against Briggs's {briggs:.0} m"
                );
            }
        }
        // A 30 MW fire, as the doc gives it: its smoke leaves at about 6.7 m/s, 50 m up after 10 s, 200 m after a
        // minute (where the game's column used to leave at 15 m/s and stand 100 m up after 10 s).
        let full = column_climb(30_000.0, 30_000.0, 0.0);
        assert!((full.0 + full.1 - 6.7).abs() < 0.3);
        assert!((climbed(full, 10.0) - 50.0).abs() < 4.0);
        assert!((climbed(full, 60.0) - 200.0).abs() < 10.0);
        // Beside two more like it, its smoke leaves the fire as fast and climbs faster once the columns have merged.
        let merged = column_climb(30_000.0, 90_000.0, 0.0);
        assert_eq!(merged.0, full.0);
        assert!(merged.1 > full.1 * 1.3);
    }

    #[test]
    fn a_fire_that_is_out_lifts_no_column() {
        // As it goes out its heat carries on from where it was, then follows its flames down to nothing.
        for at in [0.08, 0.3, 0.9] {
            assert!((heat_share(true, at, at) - heat_share(false, 0.0, at)).abs() < 1e-12);
        }
        let mut last = f64::INFINITY;
        for step in (0..=100).rev() {
            let share = heat_share(false, 0.0, f64::from(step) / 100.0);
            assert!(share <= last);
            last = share;
        }
        assert_eq!(heat_share(false, 0.0, 0.0), 0.0);
    }

    #[test]
    fn a_chained_column_stays_whole_at_rest_and_under_way() {
        for speed in [0.0, 7.2, 16.0] {
            let (_, chain) = column(true, speed);
            let mut links: Vec<&GasPuff> = chain.smoke.gas.iter().collect();
            links.sort_by(|a, b| a.state[0].total_cmp(&b.state[0]));
            let at = |link: &GasPuff| {
                DVec3::new(
                    f64::from(link.sphere[0]),
                    f64::from(link.sphere[1]),
                    f64::from(link.sphere[2]),
                )
            };
            // Each link, from the youngest to the oldest, reaches the next one along the column: no gap opens in it.
            let mut widest = 0.0f64;
            for pair in links.windows(2) {
                let gap = (at(pair[1]) - at(pair[0])).length();
                let reach = f64::from(pair[0].sphere[3] + pair[1].sphere[3]);
                widest = widest.max(gap / reach);
            }
            let top = links
                .iter()
                .map(|link| link.sphere[1] + link.sphere[3] * link.quad[1])
                .fold(0.0f32, f32::max);
            println!(
                "at {speed} m/s: {} links, the column {top:.0} m high, the widest step {widest:.2} of the links' reach",
                links.len()
            );
            assert!(widest < 1.0, "a gap in the column at {speed} m/s");
            // Under way it trails aft, bent low over her wake.
            if speed == 0.0 {
                assert!(top > 120.0, "the column reaches {top:.0} m");
            }
        }
    }

    #[test]
    fn a_column_levels_off_where_its_buoyancy_spends_itself() {
        // Briggs's final rise: the trial's 184 MW battleship fire at rest in a 9 m/s wind (8.8 m/s at her deck)
        // levels off some 600 m up, three kilometres downwind; under way at 29 kn into it, some 220 m; a 30 MW fire in
        // that wind some 200 m, a 5 MW one some 70 m. In light air the inversion holds the biggest.
        assert!((column_top(184_000.0, 8.8) - 596.0).abs() < 5.0);
        assert!((column_top(184_000.0, 23.7) - 221.0).abs() < 5.0);
        assert!((column_top(30_000.0, 8.8) - 201.0).abs() < 5.0);
        assert!((column_top(5_000.0, 8.8) - 68.0).abs() < 3.0);
        assert_eq!(column_top(184_000.0, 2.0), INVERSION);
        assert_eq!(column_top(120_000.0, 0.0), INVERSION);
        // Its smoke eases into it from below and never climbs past it, climbing ever slower.
        let p = crate::fire_batch::FireParticle {
            rise: 4.0,
            rise_time: COLUMN_RISE_TIME,
            lift: 2.0,
            top: 300.0,
            ..Default::default()
        };
        let mut last = (0.0, f64::INFINITY);
        for t in [5.0, 20.0, 60.0, 120.0, 240.0, 600.0] {
            let (climb, rising) = p.climb_at(t);
            assert!(climb > last.0 && climb < 300.0, "{climb:.0} m at {t} s");
            assert!(rising < last.1 && rising > 0.0);
            last = (climb, rising);
        }
        assert!(last.0 > 0.97 * 300.0 && last.1 < 0.05);
        // Near the fire it climbs as freely as it would with no top.
        let free = crate::fire_batch::FireParticle {
            top: f64::INFINITY,
            ..p.clone()
        };
        assert!((p.climb_at(5.0).0 / free.climb_at(5.0).0 - 1.0).abs() < 0.01);
    }

    #[test]
    fn a_columns_smoke_spreads_downwind_as_a_pall_for_minutes() {
        // Six minutes of a full fire at rest in a 9 m/s wind along +x.
        let wind = DVec3::new(9.0, 0.0, 0.0);
        let (fires, _) = burning_in(true, 0.0, 360, wind, |_| FireState {
            intensity: 1.0,
            ..Default::default()
        });
        let column = &fires.columns()[0];
        let top = column.top;
        let links: Vec<&crate::fire_batch::FireParticle> = fires
            .smoke
            .particles()
            .iter()
            .filter(|p| p.age >= 0.0 && p.age < p.life)
            .collect();
        let far = links.iter().map(|p| p.position.x).fold(0.0f64, f64::max);
        let high = links.iter().map(|p| p.position.y).fold(0.0f64, f64::max);
        let wide = links.iter().map(|p| p.extent()).fold(0.0f64, f64::max);
        // Its pall: the links more than a kilometre downwind, where it has stopped climbing.
        let pall: Vec<_> = links.iter().filter(|p| p.position.x > 1000.0).collect();
        let lowest = pall
            .iter()
            .map(|p| p.position.y)
            .fold(f64::INFINITY, f64::min);
        println!(
            "{:.0} MW, levelling off {top:.0} m over the fire: {} links live, {} of them in the pall {far:.0} m \
             downwind, {lowest:.0} to {high:.0} m up, {wide:.0} m the widest",
            column.heat / 1000.0,
            links.len(),
            pall.len()
        );
        assert!(top > 150.0 && top < INVERSION);
        // Kilometres downwind, minutes old, at the column's top and no higher, the widest some hundreds of metres.
        assert!(far > 2500.0, "the pall reaches {far:.0} m downwind");
        assert!(high < 8.0 + top * 1.01, "climbed to {high:.0} m");
        assert!(lowest > 0.5 * top, "the pall sags to {lowest:.0} m");
        assert!(wide > 100.0, "the widest link {wide:.0} m");
        // None outlives the pall's minutes.
        let oldest = links.iter().map(|p| p.life).fold(0.0f64, f64::max);
        assert!(oldest <= PALL_SECONDS, "a link lives {oldest:.0} s");
        // Few links make it: within the batch's share for one fire, a handful of them the pall.
        assert!(
            pall.len() >= 4 && links.len() < 160,
            "{} links",
            links.len()
        );
        // Each link, from the youngest to the oldest, reaches the next one along the column and its pall.
        let mut chain = links.clone();
        chain.sort_by(|a, b| a.age.total_cmp(&b.age));
        for pair in chain.windows(2) {
            let gap = (pair[1].position - pair[0].position).length();
            assert!(
                gap < 1.2 * (pair[0].extent() + pair[1].extent()),
                "a gap of {gap:.0} m at {:.0} s",
                pair[1].age
            );
        }
    }

    #[test]
    fn a_fought_fires_column_is_mottled_and_paler_as_the_hoses_gain_on_it() {
        // Its heat at `rate` per second, a party on it.
        let fought = |rate: f64| {
            move |t: f64| FireState {
                intensity: 1.0,
                heat: Some(1.2 + rate * t),
                trend: Some(FireTrend::Contained),
                suppressed: Some(true),
            }
        };
        let grey = |puff: &GasPuff| (puff.tint[0] + puff.tint[1] + puff.tint[2]) / 3.0;
        let share = |puffs: &[f32], test: fn(f32) -> bool| {
            puffs.iter().filter(|&&g| test(g)).count() as f32 / puffs.len().max(1) as f32
        };
        let (_, black) = column(true, 0.0);
        let soot = black.smoke.gas.iter().map(grey).fold(0.0f32, f32::max);
        let mut pale = Vec::new();
        let hoses_gaining = crate::EffectRules::default().hoses_gaining();
        for (what, rate) in [("losing", hoses_gaining), ("gaining", -hoses_gaining)] {
            let (_, chain) = burning(true, 0.0, fought(rate));
            let (billows, links): (Vec<&GasPuff>, Vec<&GasPuff>) = chain
                .smoke
                .gas
                .iter()
                .partition(|p| p.extra[3] == crate::instances::GAS_COLUMN);
            let links: Vec<f32> = links.into_iter().map(grey).collect();
            let billows: Vec<f32> = billows.into_iter().map(grey).collect();
            let (streaks, white, black_heads) = (
                share(&links, |g| g < 0.09),
                share(&billows, |g| g > 0.25),
                share(&billows, |g| g < 0.1),
            );
            println!(
                "hoses {what}: {} links, {:.0}% dark streaks; {} billows, {:.0}% white, {:.0}% black (an unfought column's palest {soot:.2})",
                links.len(),
                streaks * 100.0,
                billows.len(),
                white * 100.0,
                black_heads * 100.0
            );
            // Mostly grey links, paler than an unfought fire's soot (0.07 at its palest), a few dark streaks among them.
            assert!(
                share(&links, |g| g > 0.09) > 0.6,
                "hoses {what}: the links are not grey"
            );
            assert!(streaks > 0.0, "hoses {what}: no dark streaks");
            // Heads of black oil smoke and bursts of pale steam among the billows.
            assert!(
                white > 0.0 && black_heads > 0.0,
                "hoses {what}: billows of one shade"
            );
            pale.push(white);
        }
        assert!(pale[1] > pale[0], "no more steam as the hoses gain on it");
        assert!(soot < 0.12, "an unfought column is not black");
    }

    #[test]
    fn a_chained_column_draws_far_fewer_puffs_than_the_games() {
        for speed in [0.0, 16.0] {
            let (puffs, quads) = column(false, speed);
            let (links, chain) = column(true, speed);
            // The game's column is quads, the chain's is gas, never both.
            assert!(quads.smoke.gas.is_empty() && !quads.smoke.quads.is_empty());
            assert!(chain.smoke.quads.is_empty() && !chain.smoke.gas.is_empty());
            let (before, after) = (puffs.counts().smoke, links.counts().smoke);
            println!(
                "at {speed} m/s: {before} puffs alive ({} drawn, fill {:.2}); {after} links ({} drawn, fill {:.2})",
                quads.smoke.quads.len(),
                quads.smoke.fill,
                chain.smoke.gas.len(),
                chain.smoke.fill
            );
            assert!(after * 2 < before, "{after} links against {before} puffs");
            // Most of it is links of the plume; a gust's heads are billows among them, upright smoke drawn on the
            // links' path.
            let (links, billows): (Vec<&GasPuff>, Vec<&GasPuff>) = chain
                .smoke
                .gas
                .iter()
                .partition(|puff| puff.extra[2] == crate::flipbook::clip::PLUME as f32);
            assert!(!billows.is_empty() && billows.len() < links.len());
            for billow in &billows {
                assert_eq!(billow.extra[2], crate::flipbook::clip::SMOKE as f32);
                assert_eq!(billow.extra[3], crate::instances::GAS_COLUMN);
                assert!((0.0..1.0).contains(&billow.evolution[0]));
            }
            for link in &links {
                // Each plays the plume, lies along a unit axis that climbs, and is longer than the puff is wide.
                let axis = DVec3::new(
                    f64::from(link.quad[0]),
                    f64::from(link.quad[1]),
                    f64::from(link.quad[2]),
                );
                assert!((axis.length() - 1.0).abs() < 1e-3 && axis.y > 0.0);
                assert!((0.0..1.0).contains(&link.evolution[0]));
            }
            // Under way the column runs aft of her.
            if speed > 0.0 {
                let aft = links.iter().filter(|link| link.quad[0] < -0.5).count();
                assert!(aft * 2 > links.len());
            }
        }
    }

    #[test]
    fn every_fire_in_view_lights_the_scene_at_the_light_of_its_flames() {
        let (fires, camera) = burning_line();
        let distance = |light: &PointLight| (light.position - camera.position).length();
        // Every fire in view, out to the horizon, the nearest first.
        let lights = fires.lights();
        assert_eq!(lights.len(), 4);
        assert!(lights.iter().any(|light| distance(light) > 5000.0));
        assert!(
            lights
                .windows(2)
                .all(|pair| distance(&pair[0]) < distance(&pair[1]))
        );
        // No cut-off: a light reaches as far as its light does (the renderer culls it there).
        assert!(lights.iter().all(|light| light.distance == 0.0));
    }

    /// A big fire near the lens shines from among its flames as the flipbook draws them: its light split over a few
    /// points no nearer each other than [`FIRE_LIGHT_APART`], each the size of its flames, as much light in all as from
    /// the one point over its place it shines from without the flipbook's flames.
    #[test]
    fn a_big_fires_light_shines_from_among_its_flames() {
        let lit = |flipbook: bool| {
            let profile = Arc::new(ShipEffectsProfile {
                hull_length: 180.0,
                hull_beam: 20.0,
                zones: vec![Some(ZoneVent {
                    position: DVec3::new(0.0, 8.0, 0.0),
                    half: [12.0, 14.0],
                    scale: 2.0,
                })],
                ..Default::default()
            });
            let mut ship = ShipFrame::new("ship", profile, ShipPose::default());
            ship.zone_fires = vec![FireState {
                intensity: 1.0,
                ..Default::default()
            }];
            let eye = DVec3::new(0.0, 26.0, 130.0);
            let camera = CameraView {
                position: eye,
                view: glam::dcamera::rh::view::look_at_mat4(
                    eye,
                    DVec3::new(0.0, 10.0, 0.0),
                    DVec3::Y,
                ),
                projection: glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                    45f64.to_radians(),
                    16.0 / 9.0,
                    0.5,
                ),
                near: 0.5,
                fov_y_degrees: 45.0,
                height: 720.0,
                curvature: 0.0,
            };
            let mut fires = LocalizedFires::new();
            fires.flipbook = flipbook;
            for _ in 0..120 {
                fires.update(
                    1.0 / 60.0,
                    &camera,
                    DVec3::ZERO,
                    None,
                    std::slice::from_ref(&ship),
                );
            }
            fires.lights().to_vec()
        };
        let (points, single) = (lit(true), lit(false));
        assert!(
            (2..=FIRE_LIGHT_POINTS).contains(&points.len()),
            "{} points",
            points.len()
        );
        assert_eq!(single.len(), 1);
        for (i, a) in points.iter().enumerate() {
            assert!(a.radius > 1.0 && !a.marched);
            for b in &points[i + 1..] {
                assert!(a.position.distance(b.position) >= FIRE_LIGHT_APART);
            }
        }
        // Each point flickers on its own, between 0.6 and 1 of its share: the sum as the single light's within that.
        let total: f64 = points.iter().map(|light| light.intensity).sum();
        let one = single[0].intensity;
        assert!(
            total > one * 0.6 && total < one / 0.6,
            "{total} against {one}"
        );
    }

    /// A hull burning in her upperworks rolls over 40 s in and is keel up 8 s later: her fire goes out in a burst of
    /// steam on the water, its column, fed no more, clears from its foot while the smoke aloft drifts on, and nothing
    /// of it is left a minute and a half on.
    #[test]
    fn a_fires_column_is_let_go_when_its_place_goes_under() {
        let profile = Arc::new(ShipEffectsProfile {
            hull_length: 240.0,
            hull_beam: 36.0,
            zones: vec![Some(ZoneVent {
                position: DVec3::new(0.0, 14.0, 0.0),
                half: [6.0, 8.0],
                scale: 1.2,
            })],
            ..Default::default()
        });
        let eye = DVec3::new(0.0, 30.0, 400.0);
        let camera = CameraView {
            position: eye,
            view: glam::dcamera::rh::view::look_at_mat4(eye, DVec3::new(0.0, 60.0, 0.0), DVec3::Y),
            projection: glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                45f64.to_radians(),
                16.0 / 9.0,
                0.5,
            ),
            near: 0.5,
            fov_y_degrees: 45.0,
            height: 720.0,
            curvature: 0.0,
        };
        for chain in [false, true] {
            let mut fires = LocalizedFires::new();
            fires.flipbook = chain;
            fires.column = chain;
            fires.billows = chain;
            let tag = MemoryKey {
                place: Place::Zone,
                index: 0,
            }
            .tag();
            // The column's own smoke (not the steam), lowest and highest.
            let column = |fires: &LocalizedFires| {
                let heights: Vec<f64> = fires
                    .smoke
                    .particles()
                    .iter()
                    .filter(|p| p.age >= 0.0 && p.age < p.life && p.tag == tag && p.albedo.r < 0.3)
                    .map(|p| p.position.y)
                    .collect();
                let low = heights.iter().copied().fold(f64::INFINITY, f64::min);
                let high = heights.iter().copied().fold(0.0f64, f64::max);
                (heights.len(), low, high)
            };
            let steam = |fires: &LocalizedFires| {
                fires
                    .smoke
                    .particles()
                    .iter()
                    .filter(|p| p.age >= 0.0 && p.age < p.life && p.albedo.r > 0.8)
                    .map(|p| p.origin.y)
                    .collect::<Vec<f64>>()
            };
            let mut under = None;
            let mut seen = Vec::new();
            for frame in 0..180 * 60 {
                let t = f64::from(frame) / 60.0;
                // Over from 40 s to 48 s.
                let roll = std::f64::consts::PI * ((t - 40.0) / 8.0).clamp(0.0, 1.0);
                let pose = ShipPose {
                    roll,
                    y: -3.0 * (roll / std::f64::consts::PI),
                    ..ShipPose::default()
                };
                let mut ship = ShipFrame::new("ship", profile.clone(), pose);
                // The simulation keeps her fire's state keel up.
                ship.zone_fires = vec![FireState {
                    intensity: 1.0,
                    ..Default::default()
                }];
                if under.is_none() && pose.local_to_world(DVec3::new(0.0, 14.0, 0.0)).y <= 0.0 {
                    under = Some(t);
                    seen.push(("under", column(&fires)));
                }
                fires.update(
                    1.0 / 60.0,
                    &camera,
                    DVec3::new(3.0, 0.0, 0.0),
                    None,
                    std::slice::from_ref(&ship),
                );
                if let Some(at) = under {
                    for (mark, name) in
                        [(0.5, "steam"), (8.0, "8 s"), (30.0, "30 s"), (90.0, "90 s")]
                    {
                        if (t - at - mark).abs() < 0.5 / 60.0 {
                            seen.push((name, column(&fires)));
                            if name == "steam" {
                                let steam = steam(&fires);
                                assert!(!steam.is_empty(), "no steam where the fire went under");
                                assert!(
                                    steam.iter().all(|y| (0.0..2.0).contains(y)),
                                    "steam rising from {steam:?}, not the water"
                                );
                            }
                        }
                    }
                }
            }
            println!("chain {chain}: {seen:?}");
            let at = |name: &str| seen.iter().find(|(n, _)| *n == name).expect(name).1;
            let (count, low, high) = at("under");
            assert!(
                count > 0 && low < 30.0 && high > 120.0,
                "no column before she rolled"
            );
            // Fed no more: its foot clears first (the lowest smoke well up), its smoke aloft goes on.
            let (count8, low8, high8) = at("8 s");
            assert!(count8 > 0 && count8 < count, "{count8} puffs of {count}");
            assert!(
                low8 > low + 40.0,
                "its foot {low:.0} m, 8 s later {low8:.0} m"
            );
            assert!(high8 >= high * 0.9);
            let (_, low30, _) = at("30 s");
            assert!(low30 > low8);
            assert_eq!(at("90 s").0, 0, "the column still hangs there");
            assert!(
                fires.memories["ship"].zones[0].is_none(),
                "the drowned fire is remembered"
            );
            assert!(fires.lights().is_empty());
        }
    }
}
