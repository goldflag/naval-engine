//! Muzzle blasts, shell strikes, HE bursts, magazine detonations and smouldering holes: the web client's
//! `BlastEffects` (the web game's `BlastEffects.ts`), recipe for recipe and in the same order of random draws, so an
//! event draws the same puffs as in the browser; except a turret's blast, drawn since 2026-10-09 from the photographs
//! of battleships' broadsides (`turret`), and the shell strikes, drawn since 2026-10-03 from what the round did.
//!
//! Water is another lane's. The spray a turret's blast tears off the sea under its guns is naval-water-effects'
//! blast sheet: each turret's blast is handed on as a [`TurretBlast`] (`CombatEffects::turret_blasts`), which the
//! host gives the water effects. A magazine's shock ring, base surge and columns are left out. The web's water came
//! last in these recipes, so leaving it out changes no other draw.

use glam::DVec3;

use crate::debris::{Debris, DebrisBurst};
use crate::events::CombatEvent;
use crate::math::{Rgb, ShipPose, clamp, normalize, v3};
use crate::particles::{Align, EffectParticle, ParticlePool, PoolKind, SourceId};
use crate::random::Lcg;
use crate::ships::ShipFrame;

const UP: DVec3 = DVec3::Y;

/// Gunsmoke: brown-grey where the propellant burnt out in the bank, darker brown where the flash burnt it (the
/// fireball's soot). Measured on 2026-10-22 against films and photographs of Iowas firing: a second after the shot
/// the smoke is a warm brown-grey some 1.3 times as bright as her shaded hull and half as bright as the sky. The pool
/// tints its gas twice, so a stored colour is about the root of its reflectance: these reflect some 24 % and 13 %.
/// Until then they were grey `#9c9fa5` and charcoal `#85888f` (12 % and 6 %): darker than the hull and blue-grey a
/// second after the shot, near black in a large bank (a tan tried on 2026-10-09 was set aside as too orange).
fn gunsmoke() -> Rgb {
    Rgb::hex(0xbeb9b2)
}
fn gunsmoke_dark() -> Rgb {
    Rgb::hex(0xa8a197)
}
/// Smoke of burnt explosive and of burning paint, cork and fittings.
fn soot() -> Rgb {
    Rgb::hex(0x77797c)
}
fn black_smoke() -> Rgb {
    Rgb::hex(0x646669)
}

/// How high a 15-inch round's column of earth stands (m): its fastest billow thrown up the column's axis at
/// [`LAND_COLUMN_SPEED`] (some 66 m/s of it upward, the column leaning from the shot), slowed by the air (1.1/s) and
/// the earth in it pulled down at 7 m/s², tops out at about this; a smaller round's through less of the speed (a
/// 6-inch round's some 13 m).
pub const LAND_COLUMN_M: f64 = 43.0;
/// The column's fastest billow's throw, m/s per unit of calibre over 15 inches.
const LAND_COLUMN_SPEED: f64 = 68.0;

/// Smouldering holes kept at once.
const SMOULDERS: usize = 24;
/// Strikes remembered for their rounds' bursts, and for how long (s).
const HOLES: usize = 96;
const HOLE_MEMORY: f64 = 3.0;

/// An airframe coming apart in the air ([`CombatEffects::airframe_burst`](crate::CombatEffects::airframe_burst)),
/// from the renderer's own reading of its wrecks (naval-aircraft's `Wreckage`): the battle sends no event for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirframeBurst {
    pub kind: AirframeBurstKind,
    /// Where (world m) and the airframe's velocity (m/s), which what comes off it keeps for a moment.
    pub at: DVec3,
    pub velocity: DVec3,
    /// Its wingspan (m): the burst is sized by it.
    pub span: f64,
    /// Seeds the burst's random draws (the airframe's), so a replay throws the same pieces.
    pub seed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirframeBurstKind {
    /// The hits that brought her down: skin and smoke knocked off her.
    Hit,
    /// A wing or the tail torn away: fragments where it broke.
    Break,
    /// Her tanks going up as a heavy shell bursts on her: a fireball, black smoke and her pieces thrown out.
    Fireball,
}

/// A brief point light a blast asks for (`CombatEffects.illuminate`): a flash of `power` held for `hold` seconds (none
/// by default) and decaying over `duration`, and the glow of the fireball it leaves, `glow` of its power fading over
/// `glow_seconds` (none by default).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlashRequest {
    pub position: DVec3,
    pub power: f64,
    pub hold: f64,
    pub duration: f64,
    pub distance: f64,
    pub glow: f64,
    pub glow_seconds: f64,
}

/// A heavy turret's fireball, as a share of its flash's power, and how long it glows (s, × √ of the flash's size):
/// the propellant's fireball burns yellow and orange for about a fifth of a second and in pockets for a few tenths
/// more (`propellant_flame` in `gas_sprite.wgsl`), as films of battleships' broadsides show it, lighting the sea, the
/// hull beside it and its own smoke a moment after the flash.
pub(crate) const FIREBALL_GLOW: f64 = 0.2;
pub(crate) const FIREBALL_GLOW_SECONDS: f64 = 0.14;

/// A turret's flash: in its first tenth of a second the propellant gas out of its bores burns white-hot as one ball,
/// nearly round and as tall as her superstructure, as large as the fireball it turns into. Films of the Missouri's
/// broadsides from a ship alongside overexpose it at 0.03 and 0.1 s, her hull and the sea under it lit gold; by 0.2 s it
/// is a red-orange fireball mottled with soot, by a third of a second brown smoke (measured on 2026-10-22: the game's
/// ball was a third of that size, and its fireball burnt orange for half a second). The ball's diameter per unit of
/// the flash's size and of the salvo's cube root (m): an Iowa's triple 16-inch turret's 38 m, Hipper's twin 20.3 cm
/// turret's 13 m.
const FLASH_BALL: f64 = 24.0;
/// How fast it swells to its size (1/s): born at 0.7 of it, nearly all of it within 0.03 s.
const FLASH_BALL_GROWTH: f64 = 70.0;
/// How long the ball burns white and the flash's light is held at its full power (s × √ of the flash's size), before
/// both go over to the fireball's own colours: as long as the first quarter of its lobes' burning, which burns white
/// (`gas_sprite.wgsl`'s `WHITE_HEAT`).
const FLASH_HOLD: f64 = 0.1;

/// A turret's blast as the sea under its guns takes it (naval-water-effects' blast sheet): where its muzzles stood, the
/// line of fire, its calibre and how many barrels fired together.
#[derive(Clone, Debug, PartialEq)]
pub struct TurretBlast {
    pub ship_id: String,
    /// The muzzles' centre (world m) and the line of fire (unit, rising with the guns' elevation).
    pub muzzle: DVec3,
    pub bore: DVec3,
    pub caliber_m: f64,
    pub barrels: usize,
    /// The first barrel's event sequence: the sheet's own pattern is drawn from it.
    pub sequence: u64,
}

/// An explosion in the air, as its blast wave takes it (`crate::shock`): where it went off and the TNT it is worth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirBlast {
    pub at: DVec3,
    pub charge_kg: f64,
    /// The event's sequence: the front's own pattern is drawn from it.
    pub sequence: u64,
}

/// The share of a burst's charge the air outside takes when it goes off inside the hull and vents through the hole
/// its round made, up through the deck over it, or at the sea's surface (the rest goes into the ship or the water).
const VENT_SHARE: f64 = 0.15;
const DECK_SHARE: f64 = 0.35;
const WATER_SHARE: f64 = 0.5;
/// An ammunition space going up in a 240 m hull, as so much TNT at once (it goes as the cube of her length: a
/// destroyer's is a tenth of it).
pub const MAGAZINE_TNT_KG: f64 = 8000.0;
/// The pall a magazine leaves: the cloud that stands over the place after the column's fire is out, drifting with the
/// wind and spreading as smoke does. Hood's column stood "600 to 700 feet" (Phillips, Norfolk) and its pall "at least
/// 100 feet high" (Mockridge) was still standing as Prince of Wales passed it (Slade; ADM 116/4351); Queen Mary's
/// cloud rose "to a height of approximately 800 feet" (Proceedings, April 1925); Barham's "great cloud of yellowish
/// black smoke … went wreathing and eddying high into the sky" and when it cleared she was gone (Cunningham, *A
/// Sailor's Odyssey*, 1951); Mount Hood's smoke spread over some 500 yards about her (DANFS). So: billows from 40 m
/// to some 220 m over a 240 m hull ([`PALL_TOP`]), each spreading by [`PALL_SPREAD`] m/s across, standing for some
/// [`PALL_SECONDS`] and thinning over the second half of it.
const PALL_BILLOWS: f64 = 12.0;
const PALL_TOP: f64 = 220.0;
const PALL_SPREAD: f64 = 1.2;
pub const PALL_SECONDS: f64 = 600.0;
/// The pall's smoke: cordite's and fuel oil's soot, brown-black ("yellowish black", "a mass of brown smoke").
fn pall_smoke() -> Rgb {
    Rgb::hex(0x6b6359)
}
/// A magazine's own smoke, as the jets, column and head draw it: cordite's soot and fuel oil's, from grey-black to
/// brown-black ("yellowish black", Barham).
fn magazine_smoke(random: &mut Lcg) -> Rgb {
    soot().lerp(pall_smoke(), random.next())
}
/// Its wreckage's trails: cordite's and burning paint's smoke, paler than a strike's.
fn magazine_trail() -> Rgb {
    Rgb::hex(0x9a948b)
}

/// A magazine going up as films and photographs of real ones show it (Barham, 25 November 1941; Shaw at Pearl Harbor;
/// Lexington, 8 May 1942; Princeton, 24 October 1944; Mount Hood, 10 November 1944), not as a mushroom of even
/// billows. Its fireball is torn open by [`MAGAZINE_JETS`] jets of smoke flung out in a fan on every side, each a ray
/// of [`MAGAZINE_JET_BILLOWS`] billows that lengthens and tapers as it flies (Barham's first two seconds). A column of
/// [`MAGAZINE_COLUMN`] billows climbs out of a broad cloud over half her length, slowed by the air at
/// [`MAGAZINE_CLIMB_DRAG`] (1/s), so it stands at its height five to ten seconds on: some 210 to 270 m over a 240 m
/// hull (Hood's "600 to 700 feet", Queen Mary's "800 feet"), lumpy and leaning, a share [`MAGAZINE_STEAM`] of its
/// billows pale with steam. Its head is [`MAGAZINE_HEAD`] billows spreading each its own way and height, never a ring.
/// [`MAGAZINE_GRAINS`] sticks of cordite burn yellow-white for under a second, too small to see a mile off, and her
/// wreckage trails paler smoke than a strike's scraps.
const MAGAZINE_JETS: usize = 18;
const MAGAZINE_JET_BILLOWS: usize = 7;
const MAGAZINE_COLUMN: usize = 16;
const MAGAZINE_HEAD: usize = 7;
const MAGAZINE_CLIMB_DRAG: f64 = 0.3;
const MAGAZINE_STEAM: f64 = 0.2;
const MAGAZINE_GRAINS: usize = 48;

/// The charge (kg of TNT) an event's burst is worth: its shell's or bomb's filling, else one from its calibre or
/// from the reach of its blast (the simulation's reaches 3·∛kg).
fn burst_charge_kg(event: &CombatEvent) -> f64 {
    let filling = event.shell.as_ref().and_then(|shell| {
        shell
            .explosive_kg
            .filter(|kg| kg.is_finite() && *kg > 0.0)
            .or_else(|| {
                (shell.caliber_m > 0.0).then(|| crate::damage::estimated_charge_kg(shell.caliber_m))
            })
    });
    filling
        .or_else(|| event.blast_radius_m.map(|reach| (reach / 3.0).powi(3)))
        .filter(|kg| kg.is_finite() && *kg > 0.0)
        .unwrap_or(2.0)
        .min(5000.0)
}

/// The pools the blasts draw into, shared with `CombatEffects` so every gas and flash renders in the same few
/// instanced draws.
pub struct BlastPools<'a> {
    /// Gas puffs (the baked atlas relit): propellant, fireballs and explosion columns.
    pub volumes: &'a mut ParticlePool,
    /// Additive flash, flame-jet and spark sprites.
    pub fire: &'a mut ParticlePool,
    /// Flash lights requested this frame.
    pub flashes: &'a mut Vec<FlashRequest>,
}

#[derive(Clone, Debug)]
struct Smoulder {
    ship_id: SourceId,
    local: DVec3,
    normal: DVec3,
    age: f64,
    life: f64,
    credit: f64,
    strength: f64,
    seed: f64,
}

/// Where a round struck a hull, kept until it bursts (or long after it could have).
#[derive(Clone, Debug)]
struct Hole {
    shell_id: i64,
    ship_id: SourceId,
    /// The point and the plate's normal toward the shot, in the hull's frame.
    local: DVec3,
    outward: DVec3,
    /// Plating on her outside (side, deck, superstructure, a turret), not a bulkhead or deck inside.
    exterior: bool,
    /// A hole (a penetration), not a round stopped or glancing.
    holed: bool,
    scale: f64,
    age: f64,
}

/// A frame (right and up) across a direction, as `BlastEffects.frame`.
#[derive(Clone, Copy, Debug)]
struct Across {
    across: DVec3,
    vertical: DVec3,
}

impl Across {
    fn new(forward: DVec3) -> Self {
        let mut across = forward.cross(UP);
        if across.length_squared() < 1e-4 {
            across = DVec3::X;
        }
        let across = normalize(across);
        let vertical = normalize(across.cross(forward));
        Self { across, vertical }
    }

    /// A random unit direction in the plane across the forward axis.
    fn sideways(&self, random: &mut Lcg) -> DVec3 {
        let angle = random.next() * std::f64::consts::TAU;
        self.across * angle.cos() + self.vertical * angle.sin()
    }
}

/// One of the clumps a turret's blast tears its smoke into: the way it is thrown (across the line of fire, flattened
/// toward her side) and how hard, how far along the line of fire it reaches, how fast it rolls up and keeps climbing
/// (m/s, then m/s² of buoyancy on its billows) and how much of the wind it drifts with, so the clumps stand apart and
/// part further as the cloud drifts.
struct Clump {
    out: DVec3,
    throw: f64,
    reach: f64,
    roll: f64,
    climb: f64,
    wind: f64,
}

impl Clump {
    fn thrown(frame: &Across, random: &mut Lcg) -> Self {
        let out = frame.sideways(random);
        let climb = random.next();
        Self {
            out: normalize(out - frame.vertical * (out.dot(frame.vertical) * 0.5)),
            throw: 0.8 + random.next() * 1.4,
            reach: 0.6 + random.next() * 0.65,
            roll: 12.0 * climb,
            climb: 4.0 * climb * climb,
            wind: 0.45 + random.next() * 0.85,
        }
    }
}

fn volume_axis(p: &mut EffectParticle, axis: DVec3, aspect: f64) {
    p.volume_aspect = aspect;
    p.volume_yaw = axis.z.atan2(axis.x);
    p.volume_axis_y = clamp(axis.y, -1.0, 1.0);
}

/// `Vector3.reflect(normal)`.
fn reflect(v: DVec3, normal: DVec3) -> DVec3 {
    v - normal * (2.0 * v.dot(normal))
}

/// The blasts' own state: shots waiting to be grouped into turret blasts, smouldering holes and their soot.
pub struct BlastEffects {
    /// Smoke from smouldering holes: small lit gas puffs that only read the occlusion map.
    pub soot: ParticlePool,
    /// Solid pieces strikes and bursts throw (`crate::debris`).
    pub debris: Debris,
    smoulders: Vec<Smoulder>,
    holes: Vec<Hole>,
    pending: Vec<CombatEvent>,
    /// The turrets fired since [`BlastEffects::clear_fired`] (each frame's update), for the sea under their guns.
    fired: Vec<TurretBlast>,
    /// The explosions in the air since then, for their blast waves.
    bursts: Vec<AirBlast>,
    density: f64,
    /// Trial `flipbook`: fireballs and smoke emitted as simulated puffs (`flipbook_blasts.rs`).
    pub flipbook: bool,
}

impl Default for BlastEffects {
    fn default() -> Self {
        Self::new()
    }
}

impl BlastEffects {
    pub fn new() -> Self {
        Self {
            soot: ParticlePool::new(4096, PoolKind::Gas),
            debris: Debris::new(),
            smoulders: Vec::new(),
            holes: Vec::new(),
            pending: Vec::new(),
            fired: Vec::new(),
            bursts: Vec::new(),
            density: 1.0,
            flipbook: false,
        }
    }

    /// The turrets fired since the last [`BlastEffects::clear_fired`], one blast each.
    pub fn fired(&self) -> &[TurretBlast] {
        &self.fired
    }

    /// The explosions in the air since the last [`BlastEffects::clear_fired`].
    pub fn bursts(&self) -> &[AirBlast] {
        &self.bursts
    }

    pub fn clear_fired(&mut self) {
        self.fired.clear();
        self.bursts.clear();
    }

    /// A burst at the sea's surface, whose column is the water lane's: the air takes a share of its blast.
    pub fn water_burst(&mut self, event: &CombatEvent) {
        let mut at = v3(event.position);
        at.y = at.y.max(event.water_burst_y.unwrap_or(0.0));
        self.bursts.push(AirBlast {
            at,
            charge_kg: burst_charge_kg(event) * WATER_SHARE,
            sequence: event.sequence,
        });
    }

    pub fn set_density(&mut self, density: f64) {
        self.density = density;
        self.soot.density = density;
        self.debris.density = density;
    }

    /// Barrels of one salvo arrive as separate events; they become one turret blast in `flush_shots`.
    pub fn queue_shot(&mut self, event: &CombatEvent) {
        self.pending.push(event.clone());
    }

    /// Group this frame's shots by ship, tick and mount (adjacent parallel muzzles) and fire each group once.
    pub fn flush_shots(&mut self, pools: &mut BlastPools<'_>) {
        while !self.pending.is_empty() {
            let first = self.pending.remove(0);
            let calibre = first.caliber_m();
            let reach = 3.0 + calibre * 18.0;
            let direction = shot_direction(&first);
            let mut group = vec![first];
            let first = &group[0];
            let mut i = 0;
            let mut taken = Vec::new();
            while i < self.pending.len() {
                let other = &self.pending[i];
                let point = shot_direction(other);
                let near = (v3(other.position) - v3(first.position)).length() < reach;
                if other.ship_id == first.ship_id
                    && other.tick == first.tick
                    && other.caliber_m() == calibre
                    && near
                    && point.dot(direction) > 0.995
                {
                    taken.push(self.pending.remove(i));
                } else {
                    i += 1;
                }
            }
            group.extend(taken);
            let seed = group[0].sequence as f64 * 7919.0
                + group[0].shell.as_ref().map_or(0.0, |s| s.id as f64);
            let blast = turret(&group, &mut Lcg::from_number(seed), pools, self.flipbook);
            self.fired.push(blast);
        }
    }

    /// A strike on plating (`penetration`, `stopped`, `ricochet`, and `contact` remembered for its burst). Only
    /// plating on the ship's outside shows anything: a bulkhead or an armoured deck deep inside is hidden by her
    /// hull, so a strike there is only remembered.
    ///
    /// - A penetration is the moment of entry: a white spark flash, spall sprayed back off the plate's face, a puff
    ///   of burnt paint and a few flakes of it. What follows is the burst's (`burst`): an armour-piercing round goes
    ///   off a few hundredths of a second later inside her, and one that never goes off (unarmed, through light
    ///   plating) leaves its hole and nothing more.
    /// - A round stopped by armour shatters on it: a hard flash, a wide fan of sparks, scale and paint dust and hot
    ///   splinters of the shell, and no fire. An armed one bursts on the plate afterwards (`burst`).
    /// - A ricochet throws its sparks and a few splinters along the reflection and leaves a grey puff.
    pub fn impact(
        &mut self,
        event: &CombatEvent,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
        ships: &[ShipFrame],
    ) {
        let ricochet = event.kind == "ricochet";
        let stopped = event.kind == "stopped";
        let scale = clamp(event.caliber_m() / 0.38, 0.25, 1.7);
        let size = scale.sqrt();
        let incoming = normalize(
            event
                .shell
                .as_ref()
                .map_or(DVec3::new(0.0, -1.0, 0.0), |s| v3(s.velocity)),
        );
        let mut normal = normalize(v3(event.normal.unwrap_or([0.0, 1.0, 0.0])));
        // Polygon winding may face either side; the blast leaves the incoming side.
        if normal.dot(incoming) > 0.0 {
            normal = -normal;
        }
        let hull = ships.iter().find(|ship| *ship.id == *event.ship_id);
        // Her outside is the plating with open air on one side; where that side is away from the shot (a round
        // leaving her), what the strike throws goes out that way.
        let open = hull.map_or(Some(1.0), |h| {
            let local = h.pose.world_to_local(v3(event.position));
            let toward = h.pose.world_to_local(v3(event.position) + normal) - local;
            h.profile.hull.open_side(local, toward)
        });
        let exterior = open.is_some();
        if open == Some(-1.0) {
            normal = -normal;
        }
        self.remember(event, normal, exterior, scale, hull);
        if event.kind == "contact" || !exterior {
            return;
        }
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        let at = v3(event.position) + normal * 0.2;
        let out = if ricochet && normal.dot(incoming) < 0.0 {
            reflect(incoming, normal)
        } else {
            normal
        };
        let source = Some(SourceId::from(event.ship_id.as_str()));
        flashes.push(FlashRequest {
            position: at + normal * 1.5,
            power: (if ricochet {
                90.0
            } else if stopped {
                260.0
            } else {
                150.0
            }) * scale,
            duration: if stopped { 0.12 } else { 0.08 },
            distance: (if ricochet { 22.0 } else { 34.0 }) * size,
            ..Default::default()
        });
        // The flash: white-hot steel and the shell's cap, gone in a few frames.
        let flash = fire.emit(at, None);
        flash.size = (if stopped { 4.5 } else { 2.8 }) * size;
        flash.growth = (if stopped { 70.0 } else { 38.0 }) * size;
        flash.growth_decay = 20.0;
        flash.life = if stopped { 0.09 } else { 0.06 };
        flash.color = Rgb::new(6.0, 5.6, 5.0);
        flash.opacity = 1.0;
        if stopped {
            let glow = fire.emit(at, None);
            glow.size = 5.0 * size;
            glow.growth = 20.0 * size;
            glow.growth_decay = 7.0;
            glow.life = 0.16;
            glow.color = Rgb::new(3.0, 1.6, 0.6);
            glow.opacity = 0.8;
        }
        // Sparks: molten steel and the shell's shattered cap, sprayed back off the face (or along the glance).
        let frame = Across::new(out);
        let sparks = if stopped {
            54
        } else if ricochet {
            26
        } else {
            34
        };
        for _ in 0..sparks {
            let p = fire.emit(at, None);
            let spread = if stopped { 0.95 } else { 0.6 };
            let side = frame.sideways(random) * (random.next() * spread * 1.6);
            let direction = normalize(out + side);
            let speed = (14.0 + random.next().powf(0.7) * if stopped { 85.0 } else { 60.0 }) * size;
            p.velocity = direction * speed;
            p.velocity.y += random.next() * 6.0 * size;
            p.size = (0.06 + random.next() * 0.16) * size;
            p.stretch = 8.0 + random.next() * 12.0;
            p.align = Align::Velocity;
            p.life = 0.25 + random.next() * 0.65;
            p.gravity = 9.81;
            p.drag = 0.9 + random.next() * 0.8;
            p.color = Rgb::new(3.2, 2.3, 1.2).scale(0.7 + random.next() * 0.5);
            p.waterline = true;
        }
        // Burnt paint, mill scale and the shell's dust: a pale puff off the plate, browner on armour.
        for i in 0..(if stopped { 3 } else { 2 }) {
            let p = volumes.emit(at, source.as_ref());
            let side = frame.sideways(random);
            p.age = -(f64::from(i) * 0.02);
            p.position += normal * ((0.3 + random.next() * 0.4) * size);
            p.velocity = out * ((5.0 + random.next() * 7.0) * size)
                + side * ((1.0 + random.next() * 3.0) * size);
            p.velocity.y += 1.0 + random.next() * 2.0;
            p.drag = 2.4;
            p.size = (1.2 + random.next() * 1.4) * size;
            p.growth = (7.0 + random.next() * 4.0) * size;
            p.growth_decay = 1.8;
            p.diffusion = 0.35 * size;
            p.life = 2.8 + random.next() * 1.6;
            p.dissipation_time = 1.2;
            p.heat = if stopped { 0.6 } else { 0.25 };
            p.cooling = 2.5;
            p.gravity = -0.6;
            p.wind = 0.7;
            p.density = 1.9;
            p.opacity = 0.7;
            p.color = if stopped {
                Rgb::hex(0x8d877f)
            } else {
                Rgb::hex(0xa7a7a3)
            }
            .scale(0.9 + random.next() * 0.2);
        }
        // Solid bits: flakes of paint and plating off an entry, hot splinters of a shattered or glancing shell.
        let (count, speed, pieces, hot) = if stopped {
            (9, (20.0, 65.0), (0.05, 0.22), 0.85)
        } else if ricochet {
            (5, (25.0, 70.0), (0.04, 0.16), 0.7)
        } else {
            (6, (8.0, 28.0), (0.04, 0.18), 0.25)
        };
        self.debris.throw(
            &DebrisBurst {
                at,
                axis: out,
                spread: if stopped { 1.1 } else { 0.7 },
                count,
                speed: (speed.0 * size, speed.1 * size),
                size: (pieces.0 * size, pieces.1 * size),
                hot,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
    }

    /// A shell striking the land ([`CombatEvent::struck_land`]): it bursts in the earth, or buries itself and blows
    /// it out, sized by its calibre (a 15-inch round's [`LAND_COLUMN_M`]):
    ///
    /// - a flash and its light, an orange fireball and burning fragments when it is a high-explosive round;
    /// - a column of brown earth thrown up in a fan, leaning away from the shot, that slows and falls back;
    /// - clods of earth, stones and a few of the shell's splinters thrown out and up, trailing dust;
    /// - a skirt of pale dust rolling out over the ground as the column comes down;
    /// - a pall of smoke and dust left hanging over the place, drifting off with the wind.
    pub fn land_strike(
        &mut self,
        event: &CombatEvent,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
    ) {
        let k = clamp(event.caliber_m() / 0.38, 0.2, 1.25);
        let he = event
            .shell
            .as_ref()
            .is_some_and(|s| s.shell_type.as_deref() == Some("HE"));
        let at = v3(event.position);
        // The column leans away from the shot a little: earth thrown from the far side of the crater it digs.
        let incoming = event.shell.as_ref().map_or(DVec3::ZERO, |s| v3(s.velocity));
        let along = DVec3::new(incoming.x, 0.0, incoming.z).normalize_or_zero();
        let axis = normalize(UP + along * 0.22);
        let charge = burst_charge_kg(event);
        self.bursts.push(AirBlast {
            at,
            charge_kg: charge * if he { 0.5 } else { 0.25 },
            sequence: event.sequence,
        });
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        flashes.push(FlashRequest {
            position: at + UP * (3.0 * k),
            power: (if he { 1300.0 } else { 600.0 }) * k * k,
            duration: 0.12,
            distance: 55.0 * k,
            ..Default::default()
        });
        let flash = fire.emit(at, None);
        flash.size = 4.0 * k;
        flash.growth = (if he { 70.0 } else { 40.0 }) * k;
        flash.growth_decay = 14.0;
        flash.life = 0.09;
        flash.color = Rgb::new(5.0, 4.2, 3.1);
        if he {
            let glow = fire.emit(at + UP * (1.5 * k), None);
            glow.size = 6.0 * k;
            glow.growth = 26.0 * k;
            glow.growth_decay = 5.0;
            glow.life = 0.24;
            glow.color = Rgb::new(3.2, 1.45, 0.45);
            glow.opacity = 0.85;
            for _ in 0..12 {
                let p = fire.emit(at, None);
                let direction =
                    normalize(axis + Across::new(axis).sideways(random) * random.next());
                p.velocity = direction * ((20.0 + random.next() * 40.0) * k);
                p.size = (0.1 + random.next() * 0.16) * k;
                p.stretch = 6.0 + random.next() * 6.0;
                p.align = Align::Velocity;
                p.life = 0.3 + random.next() * 0.5;
                p.gravity = 9.81;
                p.drag = 0.8;
                p.color = Rgb::new(2.6, 1.5, 0.6);
                p.waterline = true;
                p.surface_y = at.y - 0.5;
            }
        }
        let source: Option<SourceId> = None;
        // The column: billows of earth thrown up in a fan, the fastest straight up its axis and the slower ones spread
        // wider, so they stand one above the next and open out as they go; each slowed by the air and pulled back down
        // by the earth in it. Brown earth, paler where the dust is.
        let billows = 11;
        for i in 0..billows {
            let p = volumes.emit(at, source.as_ref());
            let share = f64::from(i) / f64::from(billows - 1);
            let spread = 0.55 * (1.0 - share) * (0.4 + 0.6 * random.next());
            let direction = normalize(axis + Across::new(axis).sideways(random) * spread);
            p.age = -(random.next() * 0.05);
            p.position += direction * (0.5 * k);
            p.velocity = direction
                * ((0.3 + 0.7 * share) * LAND_COLUMN_SPEED * k * (0.92 + random.next() * 0.16));
            p.drag = 1.1;
            p.gravity = 7.0;
            volume_axis(p, direction, 1.6);
            p.size = (2.6 + random.next() * 1.4) * k;
            p.growth = (11.0 + random.next() * 6.0) * k;
            p.growth_decay = 1.3;
            p.diffusion = 0.6 * k;
            p.heat = if he { 0.5 } else { 0.15 };
            p.cooling = 3.0;
            p.life = 4.5 + random.next() * 1.5;
            p.dissipation_time = 1.6;
            p.wind = 0.4;
            p.density = 3.0;
            p.opacity = 0.9;
            p.color = if he {
                Rgb::hex(0x857a6c)
            } else {
                Rgb::hex(0x8c7b64)
            }
            .scale(0.85 + random.next() * 0.25);
        }
        // The skirt: dust rolling out over the ground as the column falls back.
        let skirt = 7;
        for i in 0..skirt {
            let p = volumes.emit(at, source.as_ref());
            let yaw =
                (f64::from(i) + random.next() * 0.6) / f64::from(skirt) * std::f64::consts::TAU;
            let out = DVec3::new(yaw.cos(), 0.0, yaw.sin());
            p.age = -(0.15 + random.next() * 0.3);
            p.position += out * (2.0 * k) + UP * (1.0 * k);
            p.velocity = out * ((8.0 + random.next() * 8.0) * k) + UP * (0.5 + random.next());
            p.drag = 1.4;
            p.gravity = -0.15;
            volume_axis(p, out, 1.4);
            p.size = (2.5 + random.next()) * k;
            p.growth = 8.0 * k;
            p.growth_decay = 0.5;
            p.diffusion = 0.6 * k;
            p.life = 9.0 + random.next() * 4.0;
            p.dissipation_time = 3.5;
            p.wind = 0.9;
            p.density = 2.2;
            p.opacity = 0.75;
            p.color = Rgb::hex(0xa59d90).scale(0.9 + random.next() * 0.2);
        }
        // The pall: smoke and fine dust left hanging over the place.
        for _ in 0..3 {
            let p = volumes.emit(at, source.as_ref());
            p.age = -(0.6 + random.next() * 0.6);
            p.position += UP * ((6.0 + random.next() * 8.0) * k)
                + DVec3::new(random.next() - 0.5, 0.0, random.next() - 0.5) * (6.0 * k);
            p.velocity = UP * (1.5 + random.next() * 1.5);
            p.drag = 0.8;
            p.gravity = -0.5;
            p.size = 6.0 * k;
            p.growth = 6.0 * k;
            p.growth_decay = 0.25;
            p.diffusion = 0.9 * k;
            p.life = 16.0 + random.next() * 6.0;
            p.dissipation_time = 5.0;
            p.wind = 1.0;
            p.density = 1.6;
            p.opacity = 0.6;
            p.color = if he {
                Rgb::hex(0x8b877f)
            } else {
                Rgb::hex(0x9d9282)
            }
            .scale(0.95 + random.next() * 0.1);
        }
        // Clods and stones, with a few of the shell's splinters among them.
        self.debris.throw(
            &DebrisBurst {
                at: at + UP * 0.5,
                axis,
                spread: 0.75,
                count: (28.0 * k.max(0.35)).round() as usize,
                speed: (10.0 * k.sqrt(), 38.0 * k.sqrt()),
                size: (0.06 * k, 0.45 * k),
                hot: if he { 0.5 } else { 0.2 },
                smoking: 0.35,
                trail: 0.6,
                trail_colour: Rgb::hex(0x8a7c68),
                earth: 0.85,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
    }

    /// A shell's strike kept until its burst: where it holed or met her plating, in her frame.
    fn remember(
        &mut self,
        event: &CombatEvent,
        normal: DVec3,
        exterior: bool,
        scale: f64,
        hull: Option<&ShipFrame>,
    ) {
        let (Some(shell), Some(hull)) = (&event.shell, hull) else {
            return;
        };
        let at = v3(event.position);
        let local = hull.pose.world_to_local(at);
        if self.holes.len() >= HOLES {
            self.holes.remove(0);
        }
        self.holes.push(Hole {
            shell_id: shell.id,
            ship_id: hull.id.clone(),
            local,
            outward: hull.pose.world_to_local(at + normal) - local,
            exterior,
            holed: event.kind == "penetration",
            scale,
            age: 0.0,
        });
    }

    /// A detonation (`burst`): where the round went off decides what shows.
    ///
    /// - On or just off her plating (a high-explosive round on contact, an armed armour-piercing one stopped by the
    ///   plate): the burst's fireball and black smoke blown off the plate, fragments and torn plating with it.
    /// - Inside her hull, behind a hole it made: the blast vents back out of the hole, a jet of flame and smoke
    ///   with splinters in it and the hole smouldering after; and when it went off close under the weather deck it
    ///   tears up through it, flame, planking and plating thrown high. Deep inside, out of reach of both, it shows
    ///   nothing outside (its fires and flooding are the simulation's).
    /// - Anywhere else (in the superstructure, which is all light plating, or in the open): the full burst, with
    ///   what it tore apart thrown all round.
    pub fn burst(
        &mut self,
        event: &CombatEvent,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
        ships: &[ShipFrame],
    ) {
        let s = burst_scale(event);
        let at = v3(event.position);
        let charge_kg = burst_charge_kg(event);
        let sequence = event.sequence;
        let shell_id = event.shell.as_ref().map(|shell| shell.id);
        let hull = ships.iter().find(|ship| *ship.id == *event.ship_id);
        let Some(hull) = hull else {
            self.he_burst(event, random, pools, ships);
            return;
        };
        let local = hull.pose.world_to_local(at);
        // This round's strikes on her, nearest first.
        let mut struck: Vec<(f64, Hole)> = self
            .holes
            .iter()
            .filter(|hole| Some(hole.shell_id) == shell_id && hole.ship_id == hull.id)
            .map(|hole| (hole.local.distance(local), hole.clone()))
            .collect();
        struck.sort_by(|a, b| a.0.total_cmp(&b.0));
        // On the plating: a strike outside within a couple of metres.
        if let Some((_, hole)) = struck
            .iter()
            .find(|(distance, hole)| hole.exterior && *distance < 1.6 + s)
        {
            let point = hull.pose.local_to_world(hole.local);
            let outward = normalize(hull.pose.rotate(hole.outward));
            self.he_burst_at(point, outward, s, &event.ship_id, random, pools, ships);
            self.blast_debris(point, outward, 1.25, s, 0.0, random, &event.ship_id);
            self.bursts.push(AirBlast {
                at: point + outward,
                charge_kg,
                sequence,
            });
            return;
        }
        if !hull.profile.hull.contains(local) {
            let normal = event.normal.map_or(UP, |n| normalize(v3(n)));
            self.he_burst_at(at, normal, s, &event.ship_id, random, pools, ships);
            self.blast_debris(at, UP, 1.7, s * 1.2, 0.0, random, &event.ship_id);
            self.bursts.push(AirBlast {
                at,
                charge_kg,
                sequence,
            });
            return;
        }
        // Inside her hull: out through the nearest hole it made in her side or deck, and up through the deck.
        let size = s.sqrt();
        if let Some((distance, hole)) = struck.iter().find(|(_, hole)| hole.exterior && hole.holed)
        {
            let reach = 26.0 * size;
            if *distance < reach {
                let strength = clamp(1.0 - distance / reach, 0.3, 1.0);
                self.vent(hull, hole, s, strength, random, pools);
                self.bursts.push(AirBlast {
                    at: hull.pose.local_to_world(hole.local),
                    charge_kg: charge_kg * VENT_SHARE * strength,
                    sequence,
                });
            }
        }
        let depth = 4.5 * size;
        if let Some(deck) = hull.profile.hull.deck_above(local, depth) {
            let strength = clamp(1.0 - deck / depth, 0.0, 1.0);
            if strength > 0.05 {
                let point = hull.pose.local_to_world(local + UP * deck);
                let up = normalize(hull.pose.rotate(UP));
                self.erupt(point, up, s, strength, &event.ship_id, random, pools, ships);
                self.bursts.push(AirBlast {
                    at: point,
                    charge_kg: charge_kg * DECK_SHARE * strength,
                    sequence,
                });
            }
        }
    }

    /// An internal burst blowing back out of the hole its round made: the flash lighting the plating from inside, a
    /// jet of flame that opens into sooty smoke, splinters and burning scraps, then smoke pouring from the hole.
    fn vent(
        &mut self,
        hull: &ShipFrame,
        hole: &Hole,
        s: f64,
        strength: f64,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
    ) {
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        let size = (s * hole.scale).sqrt().max(0.4) * strength.sqrt();
        let normal = normalize(hull.pose.rotate(hole.outward));
        let at = hull.pose.local_to_world(hole.local) + normal * 0.3;
        let source = Some(hull.id.clone());
        flashes.push(FlashRequest {
            position: at + normal * 2.0,
            power: 650.0 * size * size,
            duration: 0.2,
            distance: 42.0 * size,
            ..Default::default()
        });
        let flash = fire.emit(at, None);
        flash.size = 3.2 * size;
        flash.growth = 46.0 * size;
        flash.growth_decay = 12.0;
        flash.life = 0.12;
        flash.color = Rgb::new(5.0, 3.6, 2.0);
        let frame = Across::new(normal);
        // The jet: burning gas forced out of the hole, stopped by the air within a few lengths of the hull.
        for i in 0..6 {
            let p = volumes.emit(at, source.as_ref());
            let side = frame.sideways(random);
            p.age = -(f64::from(i) * 0.015);
            p.position += side * (random.next() * 0.4 * size);
            let reach = 0.5 + 0.5 * f64::from(i) / 5.0;
            p.velocity = normal * ((16.0 + random.next() * 22.0) * size * reach + 6.0);
            p.velocity += side * ((1.0 + random.next() * 5.0) * size);
            p.velocity.y += 2.0 * size;
            p.drag = 3.0;
            volume_axis(p, normal, 1.9);
            p.size = (1.6 + random.next() * 1.6) * size;
            p.growth = (16.0 + random.next() * 10.0) * size;
            p.growth_decay = 3.6;
            p.diffusion = 0.55 * size;
            p.heat = 1.85 + random.next() * 0.2;
            p.cooling = (0.3 + random.next() * 0.2) * size.sqrt();
            p.life = 6.0 + random.next() * 2.0;
            p.dissipation_time = 2.4;
            p.gravity = -1.6;
            p.wind = 0.6;
            p.density = 3.6 + random.next() * 0.6;
            p.opacity = 0.96;
            p.color = soot().scale(0.75 + random.next() * 0.3);
            if self.flipbook {
                crate::flipbook_blasts::fireball(p);
            }
        }
        // Then smoke keeps coming out of the hole and climbs the plating.
        for i in 0..5 {
            let p = volumes.emit(at, source.as_ref());
            let side = frame.sideways(random);
            p.age = -(0.18 + f64::from(i) * 0.32 + random.next() * 0.12);
            p.position += normal * ((0.8 + random.next()) * size);
            p.velocity =
                normal * ((3.5 + random.next() * 3.0) * size) + side * (random.next() * 2.0 * size);
            p.velocity.y += (4.0 + random.next() * 3.0) * size;
            p.drag = 1.1;
            p.size = (2.6 + random.next() * 1.8) * size;
            p.growth = (6.0 + random.next() * 3.0) * size;
            p.growth_decay = 0.8;
            p.diffusion = 0.6 * size;
            p.heat = 0.45;
            p.cooling = 0.45;
            p.life = 8.0 + random.next() * 3.0;
            p.dissipation_time = 3.2;
            p.fade_in = 0.2;
            p.gravity = -2.4;
            p.wind = 0.85;
            p.density = 2.8;
            p.opacity = 0.9;
            p.color = soot().scale(0.9 + random.next() * 0.25);
            if self.flipbook {
                crate::flipbook_blasts::smoke(p);
            }
        }
        // Splinters of the shell and the ship blown out with the jet.
        for _ in 0..22 {
            let p = fire.emit(at, None);
            let direction = normalize(normal + frame.sideways(random) * (random.next() * 0.7));
            p.velocity = direction * ((25.0 + random.next() * 70.0) * size);
            p.size = (0.08 + random.next() * 0.18) * size;
            p.stretch = 10.0 + random.next() * 10.0;
            p.align = Align::Velocity;
            p.life = 0.3 + random.next() * 0.6;
            p.gravity = 9.81;
            p.drag = 0.8;
            p.color = Rgb::new(3.0, 1.8, 0.7);
            p.waterline = true;
        }
        self.debris.throw(
            &DebrisBurst {
                at,
                axis: normal,
                spread: 0.6,
                count: (8.0 + 16.0 * strength) as usize,
                speed: (14.0 * size, 55.0 * size),
                size: (0.06 * size, 0.5 * size),
                hot: 0.5,
                smoking: 0.35,
                trail: 0.55,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
        let life = (14.0 + random.next() * 16.0) * size;
        self.smoulder(
            &hull.id,
            at,
            normal,
            life,
            size,
            random,
            std::slice::from_ref(hull),
        );
    }

    /// A burst close under the weather deck tearing up through it: flame, planking and plating thrown high, a
    /// column of black smoke standing over the hole.
    #[allow(clippy::too_many_arguments)]
    fn erupt(
        &mut self,
        at: DVec3,
        up: DVec3,
        s: f64,
        strength: f64,
        ship_id: &str,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
        ships: &[ShipFrame],
    ) {
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        let e = strength.sqrt();
        let size = s.sqrt() * e;
        let source = Some(SourceId::from(ship_id));
        flashes.push(FlashRequest {
            position: at + up * 3.0,
            power: 1000.0 * size * size,
            duration: 0.18,
            distance: 60.0 * size,
            ..Default::default()
        });
        let flash = fire.emit(at, None);
        flash.size = 4.0 * size;
        flash.growth = 70.0 * size;
        flash.growth_decay = 12.0;
        flash.life = 0.12;
        flash.color = Rgb::new(5.0, 4.0, 2.6);
        let frame = Across::new(up);
        // Trial `flipbook`: one simulated blast standing on the deck for the lobes.
        let lobes = if self.flipbook {
            crate::flipbook_blasts::eruption(
                volumes,
                at,
                size,
                source.as_ref(),
                black_smoke(),
                random,
            );
            0
        } else {
            6
        };
        for _ in 0..lobes {
            let p = volumes.emit(at, source.as_ref());
            let point = normalize(up * 1.2 + frame.sideways(random) * (0.3 + random.next() * 0.8));
            p.position += point * ((0.4 + random.next()) * size);
            p.velocity = point * ((14.0 + random.next() * 18.0) * size);
            p.drag = 3.2;
            volume_axis(p, point, 1.5);
            p.size = (2.2 + random.next() * 2.0) * size;
            p.growth = (28.0 + random.next() * 14.0) * size;
            p.growth_decay = 5.0;
            p.diffusion = 0.8 * size;
            p.heat = 1.9 + random.next() * 0.2;
            p.cooling = (0.3 + random.next() * 0.25) * size.sqrt();
            p.life = 7.0 + random.next() * 2.0;
            p.dissipation_time = 2.8;
            p.gravity = -1.8;
            p.wind = 0.6;
            p.density = 3.8;
            p.opacity = 0.97;
            p.color = black_smoke().scale(0.85 + random.next() * 0.3);
        }
        for i in 0..4 {
            let p = volumes.emit(at, source.as_ref());
            p.age = -(0.2 + f64::from(i) * 0.35);
            p.position += up * (2.0 * size);
            p.velocity =
                up * ((5.0 + random.next() * 4.0) * size) + frame.sideways(random) * random.next();
            p.size = (4.0 + random.next() * 2.5) * size;
            p.growth = (10.0 + random.next() * 6.0) * size;
            p.growth_decay = 1.0;
            p.diffusion = 1.0 * size;
            p.heat = 0.5;
            p.cooling = 0.5;
            p.life = 10.0 + random.next() * 3.0;
            p.dissipation_time = 3.5;
            p.drag = 1.2;
            p.gravity = -1.2;
            p.wind = 0.8;
            p.density = 3.0;
            p.opacity = 0.92;
            p.color = black_smoke().scale(1.05 + random.next() * 0.2);
            if self.flipbook {
                crate::flipbook_blasts::smoke(p);
            }
        }
        for _ in 0..24 {
            let p = fire.emit(at, None);
            let direction = normalize(up + frame.sideways(random) * (random.next() * 0.9));
            p.velocity = direction * ((30.0 + random.next() * 70.0) * size);
            p.size = (0.1 + random.next() * 0.2) * size;
            p.stretch = 8.0 + random.next() * 10.0;
            p.align = Align::Velocity;
            p.life = 0.4 + random.next() * 0.7;
            p.gravity = 9.81;
            p.drag = 0.7;
            p.color = Rgb::new(3.0, 1.7, 0.6);
            p.waterline = true;
        }
        self.debris.throw(
            &DebrisBurst {
                at,
                axis: up,
                spread: 0.75,
                count: (10.0 + 26.0 * strength) as usize,
                speed: (18.0 * size, 70.0 * size),
                size: (0.08 * size.max(0.6), 0.85 * size.max(0.6)),
                hot: 0.35,
                smoking: 0.4,
                trail: 0.75,
                timber: 0.45,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
        let life = (16.0 + random.next() * 14.0) * size;
        self.smoulder(ship_id, at, up, life, size, random, ships);
    }

    /// Plating and fittings torn off by a burst, thrown in a cone about `axis`.
    #[allow(clippy::too_many_arguments)]
    fn blast_debris(
        &mut self,
        at: DVec3,
        axis: DVec3,
        spread: f64,
        s: f64,
        timber: f64,
        random: &mut Lcg,
        ship_id: &str,
    ) {
        let size = s.sqrt();
        let source = (!ship_id.is_empty()).then(|| SourceId::from(ship_id));
        self.debris.throw(
            &DebrisBurst {
                at: at + axis * 0.3,
                axis,
                spread,
                count: (8.0 + 14.0 * size) as usize,
                speed: (20.0 * size, 75.0 * size),
                size: (0.05 * size, 0.45 * size),
                hot: 0.4,
                smoking: 0.3,
                trail: 0.5,
                timber,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
    }

    /// High-explosive and fuzed detonations: a larger, sootier fireball and black smoke.
    pub fn he_burst(
        &mut self,
        event: &CombatEvent,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
        ships: &[ShipFrame],
    ) {
        let s = burst_scale(event);
        let mut normal = event.normal.map_or(UP, |n| normalize(v3(n)));
        if let Some(shell) = &event.shell
            && normal.dot(v3(shell.velocity)) > 0.0
        {
            normal = -normal;
        }
        self.bursts.push(AirBlast {
            at: v3(event.position),
            charge_kg: burst_charge_kg(event),
            sequence: event.sequence,
        });
        self.he_burst_at(
            v3(event.position),
            normal,
            s,
            &event.ship_id,
            random,
            pools,
            ships,
        );
    }

    /// The burst at `at`, blown out over `normal`'s side, of size `s` (its blast radius over 4.5 m).
    #[allow(clippy::too_many_arguments)]
    fn he_burst_at(
        &mut self,
        at: DVec3,
        normal: DVec3,
        s: f64,
        ship_id: &str,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
        ships: &[ShipFrame],
    ) {
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        let source = Some(SourceId::from(ship_id));
        flashes.push(FlashRequest {
            position: at + normal * (2.0 * s),
            power: 1500.0 * s * s,
            duration: 0.12,
            distance: 60.0 * s,
            ..Default::default()
        });
        let flash = fire.emit(at, None);
        flash.size = 5.0 * s;
        flash.growth = 90.0 * s;
        flash.growth_decay = 14.0;
        flash.life = 0.1;
        flash.color = Rgb::new(5.0, 4.3, 3.3);
        let glow = fire.emit(at, None);
        glow.size = 8.0 * s;
        glow.growth = 34.0 * s;
        glow.growth_decay = 5.0;
        glow.life = 0.28;
        glow.color = Rgb::new(3.2, 1.45, 0.45);
        glow.opacity = 0.85;
        for _ in 0..20 {
            let p = fire.emit(at, None);
            let y = random.next() * 1.6 - 0.6;
            let yaw = random.next() * std::f64::consts::TAU;
            let r = (1.0 - (y * y).min(1.0)).sqrt();
            let direction = normalize(DVec3::new(yaw.cos() * r, y, yaw.sin() * r) + normal * 0.6);
            p.velocity = direction * ((30.0 + random.next() * 60.0) * s);
            p.size = (0.12 + random.next() * 0.2) * s;
            p.stretch = 7.0 + random.next() * 8.0;
            p.align = Align::Velocity;
            p.life = 0.25 + random.next() * 0.5;
            p.gravity = 9.81;
            p.drag = 0.8;
            p.color = Rgb::new(2.6, 1.5, 0.6);
            p.waterline = true;
        }
        // Trial `flipbook`: a simulated blast and its smoke for the lobes and puffs below.
        let (lobes, puffs) = if self.flipbook {
            crate::flipbook_blasts::burst(
                volumes,
                at,
                normal,
                s,
                source.as_ref(),
                black_smoke(),
                random,
            );
            (0, 0)
        } else {
            (7, 4)
        };
        // The fireball: lobes of burning explosive thrown out over the side facing away from the plating, whose
        // rim cools first into the black smoke such fillings leave.
        for _ in 0..lobes {
            let p = volumes.emit(at, source.as_ref());
            let yaw = random.next() * std::f64::consts::TAU;
            let y = random.next() * 1.4 - 0.4;
            let r = (1.0 - (y * y).min(1.0)).sqrt();
            let point = normalize(DVec3::new(yaw.cos() * r, y, yaw.sin() * r) + normal * 0.8);
            p.position += point * ((0.6 + random.next() * 1.6) * s);
            p.velocity = point * ((16.0 + random.next() * 20.0) * s);
            p.velocity.y += 2.5 * s;
            p.drag = 3.6;
            volume_axis(p, point, 1.3);
            p.size = (3.0 + random.next() * 2.5) * s;
            p.growth = (38.0 + random.next() * 18.0) * s;
            p.growth_decay = 5.5;
            p.diffusion = 0.8 * s;
            p.heat = 1.9 + random.next() * 0.2;
            p.cooling = (0.3 + random.next() * 0.25) * s.sqrt();
            p.life = 7.0 + random.next() * 2.0;
            p.dissipation_time = 2.8;
            p.gravity = -1.8;
            p.wind = 0.6;
            p.density = 3.8;
            p.opacity = 0.97;
            p.color = black_smoke().scale(0.85 + random.next() * 0.3);
        }
        // Black smoke rising out of it and hanging over the hit.
        for _ in 0..puffs {
            let p = volumes.emit(at, source.as_ref());
            p.age = -(0.12 + random.next() * 0.25);
            p.position += normal * (2.0 * s);
            let scatter = DVec3::new(
                random.next() - 0.5,
                random.next() * 0.6,
                random.next() - 0.5,
            );
            p.position += scatter * (6.0 * s);
            let vx = (random.next() - 0.5) * 6.0;
            let vy = 4.0 + random.next() * 5.0;
            let vz = (random.next() - 0.5) * 6.0;
            p.velocity = DVec3::new(vx, vy, vz) * s;
            p.size = (5.0 + random.next() * 3.0) * s;
            p.growth = (12.0 + random.next() * 8.0) * s;
            p.growth_decay = 1.1;
            p.diffusion = 1.1 * s;
            p.heat = 0.3;
            p.cooling = 0.6;
            p.life = 9.0 + random.next() * 3.0;
            p.dissipation_time = 3.5;
            p.drag = 1.2;
            p.gravity = -1.1;
            p.wind = 0.8;
            p.density = 3.0;
            p.opacity = 0.92;
            p.color = black_smoke().scale(1.1 + random.next() * 0.2);
        }
        if !ship_id.is_empty() {
            let life = (8.0 + random.next() * 8.0) * s.sqrt();
            self.smoulder(ship_id, at, normal, life, s, random, ships);
        }
    }

    /// An airframe coming apart in the air ([`AirframeBurst`]): the skin and smoke its killing hits knock off, the
    /// fragments where a wing or the tail tears away, or her tanks going up in a fireball that throws her apart.
    pub fn airframe(
        &mut self,
        burst: &AirframeBurst,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
    ) {
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        // Sized by her span against a single-engined carrier aircraft's 12 m.
        let s = (burst.span / 12.0).clamp(0.6, 1.6);
        let carry = burst.velocity;
        let at = burst.at;
        match burst.kind {
            AirframeBurstKind::Hit | AirframeBurstKind::Break => {
                let hit = burst.kind == AirframeBurstKind::Hit;
                // Torn skin and bits of structure, a few of them hot where a cannon shell burst.
                self.debris.throw(
                    &DebrisBurst {
                        at,
                        axis: UP,
                        spread: std::f64::consts::PI,
                        count: if hit { 12 } else { 16 },
                        speed: (4.0, if hit { 18.0 } else { 12.0 }),
                        size: (0.04, if hit { 0.22 } else { 0.38 }),
                        hot: if hit { 0.15 } else { 0.05 },
                        smoking: 0.2,
                        trail: 0.3,
                        carry: carry * 0.92,
                        ..Default::default()
                    },
                    random,
                    None,
                );
                // A puff of smoke and dust left hanging where it happened.
                for _ in 0..if hit { 2 } else { 3 } {
                    let p = volumes.emit(at, None);
                    p.position += DVec3::new(
                        random.next() - 0.5,
                        random.next() - 0.5,
                        random.next() - 0.5,
                    ) * 1.5
                        * s;
                    p.velocity = carry * (0.25 + random.next() * 0.15);
                    p.drag = 1.6;
                    p.size = (1.4 + random.next()) * s;
                    p.growth = 3.0 * s;
                    p.growth_decay = 0.8;
                    p.diffusion = 0.6;
                    p.life = 3.0 + random.next() * 1.5;
                    p.dissipation_time = 1.8;
                    p.wind = 0.7;
                    p.density = 1.6;
                    p.opacity = 0.55 + random.next() * 0.2;
                    p.color = soot().scale(0.9 + random.next() * 0.25);
                }
            }
            AirframeBurstKind::Fireball => {
                flashes.push(FlashRequest {
                    position: at,
                    power: 900.0 * s * s,
                    duration: 0.16,
                    distance: 55.0 * s,
                    ..Default::default()
                });
                let flash = fire.emit(at, None);
                flash.size = 4.0 * s;
                flash.growth = 60.0 * s;
                flash.growth_decay = 10.0;
                flash.life = 0.14;
                flash.velocity = carry * 0.8;
                flash.color = Rgb::new(4.6, 3.6, 2.4);
                let glow = fire.emit(at, None);
                glow.size = 7.0 * s;
                glow.growth = 26.0 * s;
                glow.growth_decay = 4.0;
                glow.life = 0.42;
                glow.velocity = carry * 0.6;
                glow.color = Rgb::new(3.0, 1.35, 0.4);
                glow.opacity = 0.85;
                // Flame rolling out with the fuel for its first moments, before the smoke closes over it.
                for _ in 0..7 {
                    let p = fire.emit(at, None);
                    let y = random.next() * 1.6 - 0.6;
                    let yaw = random.next() * std::f64::consts::TAU;
                    let r = (1.0 - (y * y).min(1.0)).sqrt();
                    let point = DVec3::new(yaw.cos() * r, y, yaw.sin() * r);
                    p.position += point * s;
                    p.velocity = point * ((6.0 + random.next() * 8.0) * s) + carry * 0.55;
                    p.drag = 2.0;
                    p.size = (2.2 + random.next() * 1.4) * s;
                    p.growth = 9.0 * s;
                    p.growth_decay = 2.5;
                    p.life = 0.55 + random.next() * 0.45;
                    p.opacity = 0.8;
                    p.color = Rgb::new(2.6, 1.15, 0.35);
                }
                // Burning petrol thrown out in streaks.
                for _ in 0..16 {
                    let p = fire.emit(at, None);
                    let y = random.next() * 2.0 - 1.0;
                    let yaw = random.next() * std::f64::consts::TAU;
                    let r = (1.0 - y * y).max(0.0).sqrt();
                    let direction = DVec3::new(yaw.cos() * r, y, yaw.sin() * r);
                    p.velocity = direction * ((20.0 + random.next() * 35.0) * s) + carry * 0.7;
                    p.size = (0.18 + random.next() * 0.25) * s;
                    p.stretch = 5.0 + random.next() * 6.0;
                    p.align = Align::Velocity;
                    p.life = 0.35 + random.next() * 0.55;
                    p.gravity = 9.81;
                    p.drag = 1.2;
                    p.color = Rgb::new(2.8, 1.4, 0.45);
                }
                // The fireball: her fuel burning as it is flung out of the torn tanks, rolling into black smoke that the
                // air soon stops while she falls on out of it.
                for _ in 0..9 {
                    let p = volumes.emit(at, None);
                    let y = random.next() * 1.6 - 0.6;
                    let yaw = random.next() * std::f64::consts::TAU;
                    let r = (1.0 - (y * y).min(1.0)).sqrt();
                    let point = DVec3::new(yaw.cos() * r, y, yaw.sin() * r);
                    p.position += point * ((0.5 + random.next() * 1.5) * s);
                    p.velocity = point * ((8.0 + random.next() * 12.0) * s)
                        + carry * (0.45 + random.next() * 0.2);
                    p.drag = 2.4;
                    volume_axis(p, point, 1.25);
                    p.size = (2.6 + random.next() * 1.8) * s;
                    p.growth = (20.0 + random.next() * 10.0) * s;
                    p.growth_decay = 4.0;
                    p.diffusion = 0.8 * s;
                    p.heat = 1.85 + random.next() * 0.2;
                    p.cooling = (0.35 + random.next() * 0.25) * s.sqrt();
                    p.life = 7.0 + random.next() * 2.5;
                    p.dissipation_time = 2.6;
                    p.gravity = -1.4;
                    p.wind = 0.6;
                    p.density = 3.6;
                    p.opacity = 0.96;
                    p.color = black_smoke().scale(0.8 + random.next() * 0.3);
                }
                for _ in 0..4 {
                    let p = volumes.emit(at, None);
                    p.age = -(0.1 + random.next() * 0.25);
                    p.position += DVec3::new(
                        random.next() - 0.5,
                        random.next() * 0.5,
                        random.next() - 0.5,
                    ) * 4.0
                        * s;
                    p.velocity = carry * (0.3 + random.next() * 0.15)
                        + DVec3::new(0.0, 2.0 + random.next() * 2.0, 0.0);
                    p.drag = 1.0;
                    p.size = (4.0 + random.next() * 2.5) * s;
                    p.growth = (8.0 + random.next() * 5.0) * s;
                    p.growth_decay = 1.0;
                    p.diffusion = 1.0 * s;
                    p.heat = 0.35;
                    p.cooling = 0.6;
                    p.life = 10.0 + random.next() * 3.0;
                    p.dissipation_time = 3.5;
                    p.gravity = -0.8;
                    p.wind = 0.8;
                    p.density = 2.8;
                    p.opacity = 0.9;
                    p.color = black_smoke().scale(1.0 + random.next() * 0.2);
                }
                // What she was made of, thrown every way: skin, spars, a wheel, some of it burning.
                self.debris.throw(
                    &DebrisBurst {
                        at,
                        axis: UP,
                        spread: std::f64::consts::PI,
                        count: (36.0 * s) as usize,
                        speed: (8.0, 34.0),
                        size: (0.05, 0.55),
                        hot: 0.35,
                        smoking: 0.45,
                        trail: 0.9,
                        carry: carry * 0.85,
                        ..Default::default()
                    },
                    random,
                    None,
                );
            }
        }
    }

    /// Magazine ignition: the flash and the fireball, jets flung out of it, a column climbing out of the burst to a
    /// ragged head, secondary explosions along the hull, burning cordite and wreckage on long trails, and the pall
    /// (see [`MAGAZINE_JETS`]).
    pub fn magazine(
        &mut self,
        event: &CombatEvent,
        random: &mut Lcg,
        pools: &mut BlastPools<'_>,
        ships: &[ShipFrame],
    ) {
        let BlastPools {
            volumes,
            fire,
            flashes,
        } = pools;
        let hull = ships.iter().find(|ship| *ship.id == *event.ship_id);
        let s = clamp(
            hull.map_or(240.0, |h| h.profile.hull_length) / 240.0,
            0.35,
            1.2,
        );
        let mut base = v3(event.position);
        base.y = base.y.max(3.0);
        self.bursts.push(AirBlast {
            at: base + UP * (8.0 * s),
            charge_kg: MAGAZINE_TNT_KG * s.powi(3),
            sequence: event.sequence,
        });
        let source = Some(SourceId::from(event.ship_id.as_str()));
        let tau = std::f64::consts::TAU;
        flashes.push(FlashRequest {
            position: base + UP * (40.0 * s),
            power: 900_000.0 * s * s,
            duration: 1.6,
            distance: 2200.0 * s,
            ..Default::default()
        });
        // The flash: the fireball's first tenths of a second, white-hot and no wider than the fireball it opens into.
        // The scene takes its light from the flash light above.
        for i in 0..3 {
            let i = f64::from(i);
            let flash = fire.emit(base + UP * ((8.0 + i * 14.0) * s), None);
            flash.size = (26.0 + i * 8.0) * s;
            flash.growth = (200.0 - i * 50.0) * s;
            flash.growth_decay = 8.0 + i;
            flash.life = 0.18 + i * 0.07;
            flash.age = -i * 0.04;
            flash.color = Rgb::new(5.2, 4.7, 3.9 - i * 0.7);
            flash.opacity = 1.0;
        }
        // Trial `flipbook`: simulated blasts for the fireball's lobes, simulated smoke for the billows after.
        let flipbook = self.flipbook;
        let lobes = if flipbook {
            crate::flipbook_blasts::magazine_fireball(
                volumes,
                base,
                s,
                source.as_ref(),
                black_smoke(),
                random,
            );
            0
        } else {
            16
        };
        // Fireball: a great ball of burning propellant heaving up off the hull.
        for _ in 0..lobes {
            let yaw = random.next() * tau;
            let y = 0.1 + random.next() * 0.9;
            let r = (1.0 - y * y).sqrt();
            let p = volumes.emit(base, source.as_ref());
            let point = DVec3::new(yaw.cos() * r, y, yaw.sin() * r);
            p.position += point * ((4.0 + random.next() * 14.0) * s);
            p.velocity = point * ((20.0 + random.next() * 50.0) * s);
            p.velocity.y += 24.0 * s;
            p.drag = 1.25;
            p.size = (20.0 + random.next() * 14.0) * s;
            p.growth = 58.0 * s;
            p.growth_decay = 1.1;
            p.diffusion = 1.2 * s;
            p.heat = 2.0 + random.next() * 0.2;
            p.cooling = 1.4 + random.next() * 1.6;
            p.life = 60.0 + random.next() * 14.0;
            p.dissipation_time = 40.0;
            p.density = 3.2;
            p.gravity = -2.4;
            p.wind = 0.6;
            p.opacity = 0.97;
            p.color = black_smoke().scale(0.8 + random.next() * 0.3);
        }
        // Jets: smoke and burning propellant flung out of the fireball in a fan on every side, each a ray of billows
        // thrown together along one way, the nearer slower, so it lengthens as it flies and tapers to its tip; they
        // stand about the fireball for its first second or two and roll into its cloud as they slow (Barham's film).
        for _ in 0..MAGAZINE_JETS {
            let yaw = random.next() * tau;
            let lift = 0.35 + random.next() * 1.0;
            let way = DVec3::new(yaw.cos() * lift.cos(), lift.sin(), yaw.sin() * lift.cos());
            let tip = (120.0 + random.next() * 100.0) * s;
            let drag = 1.6 + random.next() * 0.6;
            // Its billows overlap however far it reaches.
            let spacing = tip / drag / MAGAZINE_JET_BILLOWS as f64;
            let colour = magazine_smoke(random).lerp(gunsmoke_dark(), 0.35 + random.next() * 0.4);
            let width = 0.85 + random.next() * 0.3;
            for k in 1..=MAGAZINE_JET_BILLOWS {
                let along = k as f64 / MAGAZINE_JET_BILLOWS as f64;
                let p = volumes.emit(base + way * (4.0 * s), source.as_ref());
                p.velocity = way * (tip * along);
                p.drag = drag;
                p.size = ((20.0 - 12.0 * along) * s).max(1.3 * spacing) * width;
                p.growth = (18.0 - 8.0 * along) * s;
                p.growth_decay = 0.7;
                p.diffusion = 0.8 * s;
                p.heat = match k {
                    1 => 1.1,
                    2 => 0.6,
                    _ => 0.2,
                };
                p.cooling = 0.4 + random.next() * 0.4;
                p.life = 14.0 + random.next() * 8.0;
                p.dissipation_time = 7.0;
                p.density = 2.2 - 0.9 * along;
                p.gravity = -0.6;
                p.wind = 0.7;
                p.opacity = 0.93;
                p.color = colour;
                volume_axis(p, way, 2.6);
                if flipbook {
                    crate::flipbook_blasts::magazine_billow(p);
                }
            }
        }
        // The column climbs out of the burst, each billow to its own height within five to ten seconds as the air
        // slows it, leaning together; it burns low down for its first seconds.
        let lean_yaw = random.next() * tau;
        let lean = (0.05 + random.next() * 0.17).tan();
        let leaning = DVec3::new(lean_yaw.cos() * lean, 1.0, lean_yaw.sin() * lean);
        let axis = leaning.normalize();
        for i in 0..MAGAZINE_COLUMN {
            let share = (i as f64 + random.next()) / MAGAZINE_COLUMN as f64;
            let climb = (12.0 + 60.0 * share) * s * (0.88 + random.next() * 0.24);
            let spread = (5.0 + 12.0 * share) * s;
            let stray = (1.0 + 3.0 * share) * s;
            let p = volumes.emit(base, source.as_ref());
            p.position.x += (random.next() - 0.5) * 2.0 * spread;
            p.position.z += (random.next() - 0.5) * 2.0 * spread;
            p.position.y += random.next() * 8.0 * s;
            // Its foot lies along her, a broad cloud over half her length that the column climbs out of.
            let along_hull = (random.next() - 0.5) * (1.0 - share * 2.5).max(0.0);
            if let Some(hull) = hull {
                p.position += hull.pose.rotate(DVec3::new(
                    0.0,
                    0.0,
                    along_hull * hull.profile.hull_length * 0.5,
                ));
            }
            let vx = (random.next() - 0.5) * 2.0 * stray;
            let vz = (random.next() - 0.5) * 2.0 * stray;
            p.velocity = leaning * climb + DVec3::new(vx, 0.0, vz);
            p.drag = MAGAZINE_CLIMB_DRAG;
            p.size = 14.0 * (34.0f64 / 14.0).powf(random.next()) * s;
            p.growth = (7.0 + 7.0 * share) * s;
            p.growth_decay = 0.14;
            p.diffusion = 0.8 * s;
            p.heat = 1.5 - share;
            p.cooling = 1.5 + random.next() * 2.0;
            p.life = 80.0 + random.next() * 15.0;
            p.dissipation_time = 70.0;
            p.density = 2.6;
            p.gravity = -0.25;
            p.wind = 0.5 + 0.4 * share;
            p.opacity = 0.95;
            let steam = random.next() < MAGAZINE_STEAM;
            p.color = magazine_smoke(random).scale(if steam {
                1.45
            } else {
                0.85 + random.next() * 0.3
            });
            volume_axis(p, axis, 1.3);
            if flipbook {
                crate::flipbook_blasts::magazine_billow(p);
            }
        }
        // Its head: billows climbing with the column's highest and spreading as they slow, each its own way and to
        // its own height, so the top is a ragged heap, never a ring.
        for _ in 0..MAGAZINE_HEAD {
            let yaw = random.next() * tau;
            let radial = DVec3::new(yaw.cos(), 0.0, yaw.sin());
            let p = volumes.emit(base, source.as_ref());
            p.age = -(0.5 + random.next() * 1.5);
            let climb = (65.0 + random.next() * 15.0) * s;
            p.velocity = leaning * climb + radial * ((3.0 + random.next() * 17.0) * s);
            p.drag = MAGAZINE_CLIMB_DRAG;
            p.size = 24.0 * (46.0f64 / 24.0).powf(random.next()) * s;
            p.growth = (14.0 + random.next() * 8.0) * s;
            p.growth_decay = 0.14;
            p.diffusion = s;
            p.heat = 0.7;
            p.cooling = 2.5 + random.next();
            p.life = 85.0 + random.next() * 15.0;
            p.dissipation_time = 75.0;
            p.density = 2.4;
            p.gravity = -0.3;
            p.wind = 0.85;
            p.opacity = 0.94;
            p.color = magazine_smoke(random).scale(0.95 + random.next() * 0.3);
            volume_axis(p, radial, 1.35);
            if flipbook {
                crate::flipbook_blasts::magazine_billow(p);
            }
        }
        // Secondary explosions ripple along the hull.
        for _ in 0..5 {
            let delay = 0.35 + random.next() * 2.6;
            let px = (random.next() - 0.5) * 120.0 * s;
            let py = 4.0 + random.next() * 20.0 * s;
            let pz = (random.next() - 0.5) * 120.0 * s;
            let mut point = DVec3::new(px, py, pz) + base;
            if let Some(hull) = hull {
                let lx = (random.next() - 0.5) * 14.0 * s;
                let lz = (random.next() - 0.5) * (hull.profile.hull_length * 0.7);
                point = base + hull.pose.rotate(DVec3::new(lx, 0.0, lz));
                point.y = base.y + 4.0 + random.next() * 10.0;
            }
            let flash = fire.emit(point, None);
            flash.age = -delay;
            flash.size = 14.0 * s;
            flash.growth = 120.0 * s;
            flash.growth_decay = 8.0;
            flash.life = 0.3;
            flash.color = Rgb::new(4.5, 3.0, 1.5);
            for _ in 0..2 {
                let p = volumes.emit(point, source.as_ref());
                p.age = -delay;
                let vx = (random.next() - 0.5) * 20.0;
                let vy = 18.0 + random.next() * 20.0;
                let vz = (random.next() - 0.5) * 20.0;
                p.velocity = DVec3::new(vx, vy, vz) * s;
                p.drag = 1.4;
                p.size = (9.0 + random.next() * 5.0) * s;
                p.growth = 32.0 * s;
                p.growth_decay = 1.4;
                p.diffusion = 1.2 * s;
                p.heat = 1.3;
                p.cooling = 1.2 + random.next() * 0.6;
                p.life = 26.0 + random.next() * 8.0;
                p.dissipation_time = 10.0;
                p.density = 2.6;
                p.gravity = -1.6;
                p.wind = 0.7;
                p.opacity = 0.95;
                p.color = black_smoke();
                if flipbook {
                    crate::flipbook_blasts::magazine_secondary(p);
                }
            }
        }
        // Burning cordite: sticks of it flung out burning yellow-white for under a second, streaks a foot or so
        // across close to (Shaw's white streamers) and too small to see a mile off.
        for _ in 0..MAGAZINE_GRAINS {
            let p = fire.emit(base + UP * (4.0 * s), None);
            let yaw = random.next() * tau;
            let y = 0.15 + random.next() * 0.85;
            let r = (1.0 - y * y).sqrt();
            p.velocity =
                DVec3::new(yaw.cos() * r, y, yaw.sin() * r) * ((50.0 + random.next() * 110.0) * s);
            p.size = 0.2 + random.next() * 0.25;
            p.stretch = 9.0;
            p.align = Align::Velocity;
            p.life = 0.4 + random.next() * 0.7;
            p.gravity = 9.81;
            p.drag = 0.3;
            p.color = Rgb::new(5.0, 4.0, 2.4);
            p.waterline = true;
        }
        // Wreckage hurled out of her: torn plates and girders on arcs of paler smoke than a strike's, then a spray of
        // smaller scraps, planking and burning cordite. Few burn in flight; most trail the smoke of what burns on them
        // (Mount Hood's photograph).
        self.debris.throw(
            &DebrisBurst {
                at: base + UP * (4.0 * s),
                axis: UP,
                spread: 0.95,
                count: 28,
                speed: (35.0 * s, 110.0 * s),
                size: (0.8 * s, 2.6 * s),
                hot: 0.3,
                smoking: 0.75,
                trail: 1.5,
                trail_colour: magazine_trail(),
                timber: 0.15,
                plating: 0.85,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
        self.debris.throw(
            &DebrisBurst {
                at: base + UP * (4.0 * s),
                axis: UP,
                spread: 1.1,
                count: 70,
                speed: (30.0 * s, 130.0 * s),
                size: (0.15 * s, 1.0 * s),
                hot: 0.25,
                smoking: 0.45,
                trail: 1.0,
                trail_colour: magazine_trail(),
                timber: 0.25,
                plating: 0.5,
                ..Default::default()
            },
            random,
            source.as_ref(),
        );
        // The pall: billows laid through the column as it slows (2 to 8 s on), each faded in within it, that stand,
        // drift with the wind at their height and spread for minutes.
        let billows = (PALL_BILLOWS * s).round().max(6.0);
        for i in 0..billows as usize {
            let share = i as f64 / (billows - 1.0);
            let delay = 2.0 + random.next() * 6.0;
            let mut at = base + UP * ((40.0 + share * (PALL_TOP - 40.0)) * s);
            at.x += (random.next() - 0.5) * 50.0 * s;
            at.z += (random.next() - 0.5) * 50.0 * s;
            let p = volumes.emit(at, source.as_ref());
            p.age = -delay;
            p.velocity = DVec3::new(
                (random.next() - 0.5) * 2.0,
                1.0 + random.next() * 2.0,
                (random.next() - 0.5) * 2.0,
            );
            p.drag = 0.05;
            p.size = (50.0 + random.next() * 30.0) * s;
            p.diffusion = PALL_SPREAD * s;
            p.heat = 0.0;
            p.life = PALL_SECONDS * (0.85 + 0.3 * random.next());
            p.dissipation_time = PALL_SECONDS * 0.5;
            p.density = 2.0;
            p.wind = 0.9 + 0.1 * random.next();
            p.opacity = 0.9;
            p.fade_in = 4.0;
            p.color = pall_smoke().scale(0.9 + random.next() * 0.2);
            if flipbook {
                crate::flipbook_blasts::magazine_billow(p);
            }
        }
        // The shock ring, base surge and columns thrown up around the hull are water: another lane's.
    }

    #[allow(clippy::too_many_arguments)]
    fn smoulder(
        &mut self,
        ship_id: &str,
        at: DVec3,
        normal: DVec3,
        life: f64,
        strength: f64,
        random: &mut Lcg,
        ships: &[ShipFrame],
    ) {
        if ship_id.is_empty() {
            return;
        }
        let Some(hull) = ships.iter().find(|ship| &*ship.id == ship_id) else {
            return;
        };
        let local = hull.pose.world_to_local(at);
        let tip = hull.pose.world_to_local(at + normal);
        if self.smoulders.len() >= SMOULDERS {
            self.smoulders.remove(0);
        }
        self.smoulders.push(Smoulder {
            ship_id: hull.id.clone(),
            local,
            normal: tip - local,
            age: 0.0,
            life,
            credit: 0.0,
            strength: clamp(strength, 0.3, 1.7),
            seed: random.next() * 1000.0,
        });
    }

    /// Age smoke and emit smoulder puffs for this frame, following each hole on its hull's current pose; fly the
    /// debris, its trails and flames into the soot and `fire` (advanced already this frame).
    pub fn update(&mut self, dt: f64, wind: DVec3, ships: &[ShipFrame], fire: &mut ParticlePool) {
        self.soot.advance(dt, wind);
        if dt <= 0.0 {
            return;
        }
        self.debris.update(dt, wind, &mut self.soot, fire);
        for hole in &mut self.holes {
            hole.age += dt;
        }
        self.holes.retain(|hole| hole.age < HOLE_MEMORY);
        let soot_colour = soot();
        let mut i = self.smoulders.len();
        while i > 0 {
            i -= 1;
            let density = self.density;
            let s = &mut self.smoulders[i];
            s.age += dt;
            let hull = ships.iter().find(|ship| ship.id == s.ship_id);
            let Some(hull) = hull.filter(|_| s.age < s.life) else {
                self.smoulders.remove(i);
                continue;
            };
            let pose: ShipPose = hull.pose;
            let world = pose.local_to_world(s.local);
            if world.y < 0.3 {
                continue;
            }
            let vigour = 1.0 - s.age / s.life;
            let rate = (2.2 + 2.5 * vigour) * density;
            s.credit += dt * rate;
            let mut random = Lcg::from_number((s.seed + s.age * 40.0).floor());
            let normal = pose.rotate(s.normal);
            let (strength, ship_id) = (s.strength, s.ship_id.clone());
            while self.smoulders[i].credit >= 1.0 {
                self.smoulders[i].credit -= 1.0;
                let p = self.soot.emit(
                    DVec3::new(
                        world.x + normal.x * 0.6,
                        world.y + normal.y * 0.6 + 0.4,
                        world.z + normal.z * 0.6,
                    ),
                    Some(&ship_id),
                );
                p.size = (1.4 + random.next()) * strength;
                p.growth = (2.6 + random.next() * 1.4) * strength;
                p.growth_decay = 0.28;
                p.diffusion = 0.25;
                p.life = 6.0 + random.next() * 3.0;
                p.fade_in = 0.4;
                let vx = normal.x * 2.5 + (random.next() - 0.5);
                let vy = 2.5 + random.next() * 1.5 + normal.y * 2.0;
                let vz = normal.z * 2.5 + (random.next() - 0.5);
                p.velocity = DVec3::new(vx, vy, vz);
                p.drag = 0.35;
                p.gravity = -0.25;
                p.wind = 0.9;
                p.opacity = (0.34 + 0.3 * vigour) * strength.min(1.0);
                p.angle = random.next() * 6.3;
                p.spin = (random.next() - 0.5) * 0.3;
                p.color = soot_colour.scale(0.8 + random.next() * 0.4 + (1.0 - vigour) * 0.5);
            }
        }
    }

    /// Smouldering holes burning now.
    pub fn smoulders(&self) -> usize {
        self.smoulders.len()
    }

    pub fn reset(&mut self) {
        self.soot.reset();
        self.debris.reset();
        self.pending.clear();
        self.fired.clear();
        self.smoulders.clear();
        self.holes.clear();
    }
}

/// A burst's size for its fireball, smoke and debris: its blast radius over 4.5 m (the browser game's scale), drawn
/// at a little over half of that. The browser's burst for a 15 cm round stood 15–18 m high; a photograph of one
/// shows a fireball of a few metres and a cloud of eight or ten.
fn burst_scale(event: &CombatEvent) -> f64 {
    clamp(event.blast_radius_m.unwrap_or(2.0) / 4.5, 0.25, 2.2) * 0.55
}

fn shot_direction(event: &CombatEvent) -> DVec3 {
    normalize(
        event
            .shell
            .as_ref()
            .map_or(DVec3::new(0.0, 0.0, -1.0), |s| v3(s.velocity)),
    )
}

/// One turret's blast, in the stages the photographs of battleships' broadsides show:
///
/// 1. **The flash**, in the first tenth of a second: a white-hot core and a tongue of burning gas at each muzzle, a
///    blinding flare over them, the gas burning white-hot as one ball ahead of them, as tall as her superstructure for a
///    battleship's turret ([`FLASH_BALL`]), and the flash's light on the ship and the sea, held while it burns white.
/// 2. **The fireball**, to about a fifth of a second: the fuel-rich propellant gas shot out of each bore burns as it mixes
///    with the air (the secondary flash), each parcel stopped by the air at its own distance, so the flame reaches out
///    along the line of fire into one fireball far larger than the turret. White-hot with the ball for its first tenth
///    of a second, it burns yellow and orange through most of its gas, then red in pockets mottled with its fresh black
///    soot (`propellant_flame` and `fresh_soot` in `gas_sprite.wgsl`), and the barrels of a salvo burn as one, larger as
///    the cube root of their charge.
/// 3. **Its smoke**, by a third of a second: the flame's last pockets go out, leaving its dark brown smoke, paling as it
///    mixes with the air.
/// 4. **The cloud**, for tens of seconds: the bank of propellant smoke thrown out ahead of the guns stops, billows,
///    spreads over the sea and drifts downwind as it thins.
///
/// Burning grains of propellant arc out ahead, and once the gas is gone the hot bores breathe wisps of smoke. The blast
/// on the sea under the guns is naval-water-effects' sheet of spray: the returned [`TurretBlast`].
fn turret(
    group: &[CombatEvent],
    random: &mut Lcg,
    pools: &mut BlastPools<'_>,
    flipbook: bool,
) -> TurretBlast {
    let BlastPools {
        volumes,
        fire,
        flashes,
    } = pools;
    let calibre = group[0].caliber_m();
    let scale = clamp(calibre / 0.38, 0.1, 1.7);
    let size = scale.powf(0.8);
    let big = scale > 0.45;
    // The flash grows with the charge burnt, far faster than with the bore.
    let flash = scale.powf(1.35);
    let mut forward = shot_direction(&group[0]);
    if forward.length_squared() == 0.0 {
        forward = DVec3::new(0.0, 0.0, -1.0);
    }
    let frame = Across::new(forward);
    let barrels = group.len() as f64;
    // Barrels fired together burn as one fireball, its size growing as the cube root of the charge (Hopkinson–Cranz).
    let salvo = barrels.cbrt();
    let mut center = DVec3::ZERO;
    for event in group {
        center.x += event.position[0] / barrels;
        center.y += event.position[1] / barrels;
        center.z += event.position[2] / barrels;
    }
    let source = Some(SourceId::from(group[0].ship_id.as_str()));
    // The flash: the bores' gas burning white-hot as one ball ahead of the muzzles, its light on the ship and the sea
    // held while it burns white.
    let ball = FLASH_BALL * flash * salvo;
    let middle = center + forward * (0.5 * ball) + UP * (0.1 * ball);
    let hold = FLASH_HOLD * flash.sqrt();
    flashes.push(FlashRequest {
        position: middle,
        power: 3500.0 * size * size * barrels.sqrt(),
        hold,
        duration: if big { 0.22 } else { 0.12 },
        distance: 120.0 * size,
        glow: if big { FIREBALL_GLOW } else { 0.0 },
        glow_seconds: FIREBALL_GLOW_SECONDS * flash.sqrt(),
    });
    let white = volumes.emit(middle, source.as_ref());
    white.size = 0.7 * ball;
    white.growth = 0.3 * ball * FLASH_BALL_GROWTH;
    white.growth_decay = FLASH_BALL_GROWTH;
    // Burning white through `hold` (its burning still to come over `WHITE_HEAT`), then yellow to red as it thins away
    // behind the fireball's flame.
    white.propellant = true;
    white.fireball = true;
    white.heat = 2.0;
    white.cooling = 2.2 * hold;
    white.life = white.cooling + 0.01;
    white.density = 4.0;
    white.opacity = 1.0;
    white.color = gunsmoke_dark();
    for event in group {
        let point = v3(event.position);
        // A white-hot core at the muzzle for a frame or two.
        let core = fire.emit(point + forward * (1.5 * size), None);
        core.size = 4.0 * flash + 1.5 * size;
        core.growth = 90.0 * flash;
        core.growth_decay = 20.0;
        core.life = 0.08;
        core.color = Rgb::new(5.0, 4.6, 3.8);
        core.opacity = 1.0;
        // The flame tongue: burning gas driven out along the bore, joined to the muzzle for a tenth of a second.
        for j in 0..6 {
            let j = f64::from(j);
            let jet = fire.emit(point + forward * ((1.0 + j * 0.6) * size), None);
            jet.age = -j * 0.018;
            jet.velocity = forward * ((170.0 + random.next() * 60.0) * flash + 40.0 * size);
            jet.drag = 9.0;
            jet.align = Align::Velocity;
            jet.size = (3.2 + random.next()) * (flash * 0.7 + size * 0.3);
            jet.growth = 26.0 * flash;
            jet.growth_decay = 9.0;
            jet.stretch = 4.5 + random.next() * 2.0;
            jet.life = 0.16 + random.next() * 0.06;
            jet.opacity = 0.95;
            jet.color = Rgb::new(3.6, 2.1 - j * 0.1, 0.8 - j * 0.08);
        }
        // The secondary flash, shot out of the bore: the air stops each parcel at its own distance (launch speed
        // ÷ drag), so the flame reaches out of the barrel along the line of fire into one connected fireball.
        let lobes = if big { 6 } else { 2 };
        for i in 0..lobes {
            let t = if lobes > 1 {
                f64::from(i) / f64::from(lobes - 1)
            } else {
                1.0
            };
            let side = frame.sideways(random);
            let p = volumes.emit(point + forward * ((1.5 + t) * size), source.as_ref());
            let reach = (3.0 + 30.0 * t + random.next() * 4.0) * flash;
            // And thrown aside of the line by the turbulence, the head furthest, so the fireball is lobed.
            let aside = (2.0 + random.next() * 6.0) * flash * (0.2 + t);
            // The air stops it within a tenth of a second or so: the fireball stands at its full reach while it burns.
            p.drag = 10.0 + random.next() * 2.0;
            p.velocity = (forward * reach + side * aside) * p.drag;
            p.velocity.y += (0.5 + 3.0 * t) * flash;
            // A tongue of flame along the bore at the muzzle, rounding out into the fireball's head.
            volume_axis(p, forward, 2.4 - t * 1.25);
            // Narrow at the muzzle, widest at the head: the flame opens out of the bore like a cone, and a salvo's
            // heads swell into one fireball.
            let swell = 1.0 + (salvo - 1.0) * t;
            p.size = ((2.5 + 10.0 * t) * flash + size) * swell;
            p.growth = (60.0 + 280.0 * t + random.next() * 45.0) * flash * swell;
            p.growth_decay = 8.0;
            p.diffusion = flash;
            // The propellant burns white-hot for its first tenth of a second (with the flash's ball), yellow and orange
            // through most of its gas to about 0.15 s, then red in pockets mottled with its fresh soot, the head of
            // the flame longest, until its cooling is spent: a third of a second for a battleship's guns (until
            // 2026-10-24 it burnt orange to half a second, the films' 0.2 s look at 0.35 s).
            p.propellant = true;
            p.fireball = true;
            p.heat = 1.25 + random.next() * 0.1;
            p.cooling = (0.12 + 0.14 * t + random.next() * 0.1) * flash.sqrt();
            // Its sooty smoke outlives the flame by a few seconds, rolling into the bank's (until 2026-10-23 it stood
            // as a dark ball for seven to twelve).
            p.life = if big {
                3.5 + 1.5 * t + random.next()
            } else {
                1.8 + random.next() * 0.7
            };
            p.dissipation_time = if big { 1.6 } else { 0.8 };
            p.density = 2.2;
            p.gravity = -1.5;
            p.wind = 0.7;
            p.opacity = 0.97;
            p.color =
                (if big { gunsmoke_dark() } else { gunsmoke() }).scale(0.85 + random.next() * 0.25);
            if flipbook {
                crate::flipbook_blasts::fireball(p);
            }
        }
        // Once the gas is gone the hot bore breathes a wisp of smoke.
        let wisp = volumes.emit(point - forward * (0.5 * size), source.as_ref());
        wisp.velocity = forward * (3.0 * size);
        wisp.velocity.y += 0.8;
        wisp.drag = 1.2;
        wisp.size = 1.4 * size;
        wisp.growth = 2.4 * size;
        wisp.growth_decay = 0.5;
        wisp.diffusion = 0.2 * size;
        wisp.life = 4.0 + random.next() * 2.0;
        wisp.age = -(0.35 + random.next() * 0.25);
        wisp.fade_in = 0.8;
        wisp.cooling = 0.01;
        wisp.dissipation_time = 2.2;
        wisp.density = 1.6;
        wisp.gravity = -0.8;
        wisp.wind = 1.0;
        wisp.opacity = 0.5;
        wisp.propellant = true;
        wisp.color = gunsmoke().scale(1.15);
    }
    // A blinding flare at the muzzles over the first frames.
    let flare = fire.emit(center + forward * (5.0 * flash), None);
    flare.size = 12.0 * flash * barrels.sqrt();
    flare.growth = 200.0 * flash;
    flare.growth_decay = 16.0;
    flare.life = 0.1;
    flare.color = Rgb::new(2.6, 2.1, 1.3);
    flare.opacity = 0.7;
    // The cloud: the propellant's smoke thrown out ahead of the guns, each billow stopped by the air at its own
    // distance, so the bank stretches far ahead of the turret and spreads along her side more than up or down, rolls
    // over, then hangs over the sea and drifts downwind, thinning for tens of seconds. The blast tears it into a few
    // clumps, each thrown its own way and climbing and drifting at its own pace, of billows large and small: the cloud
    // comes apart into ragged masses as it drifts rather than hanging as one rounded bank.
    let billows = if big {
        4.0 + 2.0 * barrels
    } else {
        1.0 + barrels
    } as usize;
    let clumps: Vec<Clump> = (0..if big { 3 } else { 2 })
        .map(|_| Clump::thrown(&frame, random))
        .collect();
    for i in 0..billows {
        let t = if billows > 1 {
            (i as f64 / (billows - 1) as f64 * 0.85 + random.next() * 0.2).min(1.0)
        } else {
            0.5
        };
        let clump = &clumps[(random.next() * clumps.len() as f64) as usize % clumps.len()];
        let out = normalize(clump.out + frame.sideways(random) * 0.45);
        let along = (3.0 + random.next() * 4.0) * size;
        let aside = random.next() * 3.0 * size;
        let p = volumes.emit(center + forward * along + out * aside, source.as_ref());
        // A heavy turret's cloud is some 60–80 m long two seconds after the shot (the Missouri's three 16-inch guns
        // filmed from a ship alongside: 60–90 m) and its dense part thins within three to five seconds, leaving a haze
        // that drifts off. Until 2026-10-23 its billows grew to 45–100 m each and stayed dense for ten seconds, a bank
        // twice the size and three times as long-lived as the films'.
        let reach = (6.0 + t * t * 45.0 * clump.reach + random.next() * 6.0) * size;
        p.drag = 2.4 + random.next() * 0.6;
        p.velocity = forward * (reach * p.drag);
        p.velocity += out * ((6.0 + random.next() * 12.0) * size * clump.throw);
        p.velocity.y += 1.0 + random.next() * 4.0 + clump.roll;
        volume_axis(p, forward, 1.5 - t * 0.3);
        // Mostly middling billows and a few great ones.
        let r = random.next();
        p.size = (6.0 + 11.0 * r * r) * size * (1.0 - t * 0.15);
        p.growth = (12.0 + random.next() * 14.0) * size;
        p.growth_decay = 0.9;
        p.diffusion = (0.3 + random.next() * 0.5) * size;
        // Nearest the muzzles the gas burnt hottest: pockets of flame deep in the cloud for a moment.
        p.propellant = true;
        p.heat = (1.0 - t) * (0.6 + random.next() * 0.3);
        p.cooling = (0.15 + random.next() * 0.1) * size.sqrt();
        p.life = if big {
            12.0 + random.next() * 6.0
        } else {
            4.0 + random.next() * 1.5
        };
        p.dissipation_time = if big { 3.5 + random.next() * 1.5 } else { 1.6 };
        // Dense, so its billows stand out crisp against the sky and the sea as the photographs show them.
        p.density = (0.75 + random.next() * 0.45) * if big { 1.3 } else { 0.7 };
        p.gravity = -0.3 - random.next() * 0.4 - clump.climb;
        p.wind = clump.wind * (0.9 + random.next() * 0.2);
        p.opacity = if big { 0.92 } else { 0.8 };
        p.age = -(0.18 + t * 0.3) * size.sqrt();
        p.fade_in = 0.45;
        let colour = if t < 0.3 && big {
            gunsmoke_dark()
        } else {
            gunsmoke()
        };
        p.color = colour.scale((0.78 + random.next() * 0.42) * if big { 1.0 } else { 1.25 });
        if flipbook {
            crate::flipbook_blasts::smoke(p);
        }
    }
    // And small puffs thrown out past it, which drift off on their own: its ragged edge.
    let strays = if big { 3 + group.len() } else { 1 };
    for _ in 0..strays {
        let clump = &clumps[(random.next() * clumps.len() as f64) as usize % clumps.len()];
        let out = normalize(clump.out + frame.sideways(random) * 0.8);
        let p = volumes.emit(
            center + forward * ((4.0 + random.next() * 4.0) * size),
            source.as_ref(),
        );
        p.drag = 2.0 + random.next();
        p.velocity = forward * ((14.0 + random.next() * 40.0) * size * p.drag);
        p.velocity += out * ((12.0 + random.next() * 18.0) * size);
        p.velocity.y += 2.0 + random.next() * 6.0 + clump.roll;
        p.size = (3.0 + random.next() * 3.5) * size;
        p.growth = (6.0 + random.next() * 6.0) * size;
        p.growth_decay = 0.9;
        p.diffusion = 0.4 * size;
        p.propellant = true;
        p.life = if big {
            7.0 + random.next() * 5.0
        } else {
            3.0 + random.next() * 1.5
        };
        p.dissipation_time = if big { 3.0 } else { 1.4 };
        p.density = (0.7 + random.next() * 0.4) * if big { 1.3 } else { 0.7 };
        p.gravity = -0.3 - random.next() * 0.8 - clump.climb;
        p.wind = clump.wind * (0.8 + random.next() * 0.5);
        p.opacity = if big { 0.85 } else { 0.7 };
        p.age = -(0.25 + random.next() * 0.3) * size.sqrt();
        p.fade_in = 0.45;
        p.color = gunsmoke().scale((0.85 + random.next() * 0.3) * if big { 1.0 } else { 1.25 });
        if flipbook {
            crate::flipbook_blasts::smoke(p);
        }
    }
    // Burning propellant grains.
    let grains = ((if big { 12.0 } else { 3.0 }) * size * barrels.sqrt()).round() as usize;
    for _ in 0..grains {
        let ember = fire.emit(center + forward * (6.0 * size), None);
        let speed = (70.0 + random.next() * 90.0) * size;
        let side = frame.sideways(random);
        let spread = (10.0 + random.next() * 30.0) * size;
        ember.velocity = forward * speed + side * spread;
        ember.gravity = 9.81;
        ember.drag = 0.7;
        ember.align = Align::Velocity;
        ember.size = (0.22 + random.next() * 0.25) * size;
        ember.stretch = 5.0;
        ember.life = 0.35 + random.next() * 0.5;
        ember.color = Rgb::new(2.6, 1.3, 0.4);
        ember.opacity = 0.9;
        ember.waterline = true;
    }
    // The blast striking the sea below the guns is water's: naval-water-effects' blast sheet.
    TurretBlast {
        ship_id: group[0].ship_id.clone(),
        muzzle: center,
        bore: forward,
        caliber_m: calibre,
        barrels: group.len(),
        sequence: group[0].sequence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 240 m hull's magazine in still air: its first second is a fireball torn open by rays flung out on every side,
    /// two seconds on nothing stands much over the fireball (no cap thrown up into a mushroom), and its column climbs
    /// to its height over the next seconds, above 200 m ten seconds on.
    #[test]
    fn a_magazine_bursts_in_rays_and_its_column_climbs_after() {
        let mut blasts = BlastEffects::new();
        let mut volumes = ParticlePool::new(1024, PoolKind::Gas);
        let mut fire = ParticlePool::new(1024, PoolKind::Additive);
        let mut flashes = Vec::new();
        let event = CombatEvent {
            sequence: 1,
            kind: "module".into(),
            ship_id: "kgv".into(),
            position: [0.0, 5.0, 0.0],
            detonation: Some(true),
            ..Default::default()
        };
        blasts.magazine(
            &event,
            &mut Lcg::new(1),
            &mut BlastPools {
                volumes: &mut volumes,
                fire: &mut fire,
                flashes: &mut flashes,
            },
            &[],
        );
        // What of the blast is out (the pall's billows, laid seconds on, left aside).
        fn burst(pool: &ParticlePool) -> Vec<&EffectParticle> {
            pool.particles()
                .iter()
                .filter(|p| p.age >= 0.0 && p.age < p.life && p.life < 300.0)
                .collect()
        }
        let mut t = 0.0;
        let mut step = |seconds: f64, pool: &mut ParticlePool| {
            while t < seconds - 1e-9 {
                pool.advance(0.05, DVec3::ZERO);
                t += 0.05;
            }
        };
        step(1.0, &mut volumes);
        let out = burst(&volumes)
            .iter()
            .filter(|p| p.position.x.hypot(p.position.z) > 50.0)
            .count();
        assert!(out >= 8, "{out} billows over 50 m out a second on");
        step(2.0, &mut volumes);
        let top = |pool: &ParticlePool| {
            burst(pool)
                .iter()
                .map(|p| p.position.y)
                .fold(0.0f64, f64::max)
        };
        let early = top(&volumes);
        assert!(early < 170.0, "{early:.0} m up two seconds on");
        step(10.0, &mut volumes);
        let risen = top(&volumes);
        assert!(risen > 200.0, "{risen:.0} m up ten seconds on");
    }

    /// A battleship's magazine going up in an 8 m/s wind: its column and cap are gone within a few minutes, its pall
    /// stands for eight, drifting downwind at its height and spreading, and is gone after twelve; its wreckage is torn
    /// plating and girders.
    #[test]
    fn a_magazines_pall_stands_for_minutes_and_drifts_downwind() {
        let mut blasts = BlastEffects::new();
        let mut volumes = ParticlePool::new(1024, PoolKind::Gas);
        let mut fire = ParticlePool::new(1024, PoolKind::Additive);
        let mut flashes = Vec::new();
        let event = CombatEvent {
            sequence: 1,
            kind: "module".into(),
            ship_id: "kgv".into(),
            position: [0.0, 5.0, 0.0],
            detonation: Some(true),
            ..Default::default()
        };
        blasts.magazine(
            &event,
            &mut Lcg::new(1),
            &mut BlastPools {
                volumes: &mut volumes,
                fire: &mut fire,
                flashes: &mut flashes,
            },
            &[],
        );
        let plates = blasts.debris.count_shaped(crate::flotsam::shape::PLATE);
        let girders = blasts.debris.count_shaped(crate::flotsam::shape::GIRDER);
        assert!(
            plates > 10 && girders > 4,
            "{plates} plates, {girders} girders"
        );
        let wind = DVec3::new(8.0, 0.0, 0.0);
        // The pall's billows (living minutes) and everything else of the blast still alive.
        fn census(pool: &ParticlePool) -> (usize, Vec<&EffectParticle>) {
            let alive: Vec<&EffectParticle> = pool
                .particles()
                .iter()
                .filter(|p| p.age >= 0.0 && p.age < p.life)
                .collect();
            let pall: Vec<&EffectParticle> =
                alive.iter().copied().filter(|p| p.life > 300.0).collect();
            (alive.len() - pall.len(), pall)
        }
        let mut t = 0.0;
        let step = |seconds: f64, pool: &mut ParticlePool, t: &mut f64| {
            while *t < seconds {
                pool.advance(0.1, wind);
                *t += 0.1;
            }
        };
        step(60.0, &mut volumes, &mut t);
        let (rest, pall) = census(&volumes);
        assert_eq!(pall.len(), 12);
        assert!(rest > 0, "the column and cap stand a minute on");
        // From 40 m up to the column's top.
        let high = pall.iter().map(|p| p.position.y).fold(0.0f64, f64::max);
        let low = pall
            .iter()
            .map(|p| p.position.y)
            .fold(f64::INFINITY, f64::min);
        assert!(
            low > 30.0 && high > 200.0 && high < 450.0,
            "{low:.0} to {high:.0} m"
        );
        step(200.0, &mut volumes, &mut t);
        let (rest, pall) = census(&volumes);
        assert_eq!(rest, 0, "the column and cap are gone");
        assert_eq!(pall.len(), 12, "the pall stands");
        // Carried downwind at the pace of the wind at its height (0.9 to 1 of it: a third more than the 10 m wind's
        // from 140 m up), and spread to well over a hundred metres across.
        let pace = pall
            .iter()
            .map(|p| p.position.x / (8.0 * 195.0 * crate::combat::wind_share(p.position.y)))
            .sum::<f64>()
            / 12.0;
        assert!((pace - 0.95).abs() < 0.1, "{pace:.2} of the wind's pace");
        assert!(pall.iter().all(|p| p.current_size() > 250.0));
        step(480.0, &mut volumes, &mut t);
        assert_eq!(
            census(&volumes).1.len(),
            12,
            "still standing at eight minutes"
        );
        step(720.0, &mut volumes, &mut t);
        assert!(census(&volumes).1.is_empty(), "gone by twelve");
    }

    /// A 15-inch round striking a hillside 60 m up: no fan of sparks off steel, a column of earth whose highest billow
    /// tops out about [`LAND_COLUMN_M`] over the strike, clods thrown out and down, and dust hanging there ten
    /// seconds on; a 6-inch round's column a third as high.
    #[test]
    fn a_shell_on_the_land_throws_up_earth() {
        use crate::events::ShellInfo;
        let strike = |caliber_m: f64| {
            let mut blasts = BlastEffects::new();
            let mut volumes = ParticlePool::new(1024, PoolKind::Gas);
            let mut fire = ParticlePool::new(1024, PoolKind::Additive);
            let mut flashes = Vec::new();
            let event = CombatEvent {
                sequence: 1,
                kind: "stopped".into(),
                position: [0.0, 60.0, 0.0],
                normal: Some([0.0, 1.0, 0.0]),
                shell: Some(ShellInfo {
                    id: 7,
                    caliber_m,
                    velocity: [400.0, -300.0, 0.0],
                    shell_type: Some("AP".into()),
                    explosive_kg: None,
                }),
                ..Default::default()
            };
            assert!(event.struck_land());
            blasts.land_strike(
                &event,
                &mut Lcg::new(1),
                &mut BlastPools {
                    volumes: &mut volumes,
                    fire: &mut fire,
                    flashes: &mut flashes,
                },
            );
            assert_eq!(flashes.len(), 1);
            assert!(fire.alive() <= 2, "{} sparks", fire.alive());
            assert!(
                blasts.debris.alive() >= 10,
                "{} clods",
                blasts.debris.alive()
            );
            let mut top: f64 = 0.0;
            for _ in 0..80 {
                volumes.advance(0.05, DVec3::ZERO);
                top = volumes
                    .particles()
                    .iter()
                    .filter(|p| p.age >= 0.0 && p.age < p.life)
                    .map(|p| p.position.y - 60.0)
                    .fold(top, f64::max);
            }
            for _ in 0..120 {
                volumes.advance(0.05, DVec3::ZERO);
            }
            (top, volumes.alive())
        };
        let (heavy, left) = strike(0.381);
        assert!((heavy - LAND_COLUMN_M).abs() < 4.0, "{heavy:.1} m");
        assert!(left >= 3, "{left} puffs of dust ten seconds on");
        let (light, _) = strike(0.152);
        assert!((10.0..17.0).contains(&light), "{light:.1} m");
    }
}
