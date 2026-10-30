//! Oil on the water: the slick a hull trails while her holed bunkers leak (native only; no TypeScript source). The
//! simulation publishes the oil she is losing (`ShipVisual::oil_leak_m3s`, m³/s, from `naval_sim::bunkers`); her trail
//! ([`crate::trail::WakeFoam`]) lays what goes over the side along her track as patches and keeps them far longer than
//! her wake: each spreads wider, thins and weathers as it ages and drifts downwind with the surface, for
//! [`OIL_LIFETIME`].
//!
//! A patch's film is the oil she puts over the side per area of her track: her leak over her speed and the breadth it
//! lies across, so a hull leaking fast or barely moving lays thick oil (0.3 m³/s from a Bismarck at 21 m/s lies 0.45 mm
//! thick astern of her, a litre a second 1.5 µm). The patches overlap along her track and the atlas keeps the thickest
//! film where they do, so each holds the film there, not a share of it. Most of the oil stays in a core that spreads
//! slowly and goes, as it weathers, into the sheen about it: a film a few hundredths of a micron thick that spreads wider
//! and outlasts it. The surface reads a film by its thickness, as the Bonn Agreement's appearance code does
//! (`shaders/oil.wgsl`): the oil's own dark brown-black above about 50 µm, metallic below it, a rainbow from 5 µm down to
//! a tenth of a micron, then a sheen; any of it stills the ripples and the breaking, so the slick reads as smooth water
//! further back.
//!
//! A lost ship's own oil comes up after she has gone (the host adds it to her leak: `naval-client`'s effects, from her
//! bunkers), and where a magazine or her fires meet it, it burns ([`OilSlick::ignite`]): heavy fuel oil lights only
//! where it lies some ten millimetres thick (Buist, Interspill 2004: Bunker C and IFO 380), burns down at about a
//! millimetre a minute (Fingas, *In-situ burning of oil spills*, ARPEL 2007: heavy oils) and goes out at about a
//! millimetre of residue; the fire spreads into the thick oil about it. [`OilSlick::fires`] hands the burning patches
//! to the combat effects, which draw their flames and black smoke.
//!
//! Each hull with oil on the water takes a tile of the oil atlas ([`OIL_TILES`]² tiles of [`OIL_RESOLUTION`]² texels,
//! `Rg8Unorm`): red holds the film on a square of [`OIL_NEAR_EXTENT`] about her newest oil (fine texels for the thick
//! oil she has just put over the side), green on a square of [`OIL_FAR_EXTENT`] framed on her slick. Both follow the
//! slick, not her: a slick she has left lies where it is however far she goes. A texel holds the film's thickness on a
//! log scale ([`encode`]); the painter's max blending keeps the thickest film where patches overlap.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::trail::{EMPTY, Motion, NO_BAND, NO_START, Stamp, StampCollector, WakeHull};

/// Edge (m) of each slick's fine square, centred on its newest oil: her after body while she leaks.
pub const OIL_NEAR_EXTENT: f64 = 2048.0;
/// Edge (m) of each slick's wide square, framed on her slick: some six minutes of a straight run at full speed.
pub const OIL_FAR_EXTENT: f64 = 8192.0;
/// Texels a tile's edge (both squares: 4 m near, 16 m wide).
pub const OIL_RESOLUTION: u32 = 512;
/// Atlas tiles per edge, and the slicks it holds (the focus's and the nearest hulls' first).
pub const OIL_TILES: usize = 3;
pub const OIL_SLOTS: usize = OIL_TILES * OIL_TILES;
/// The film's thickness (m) at the bottom and top of the atlas's scale ([`encode`]).
pub const THINNEST_M: f64 = 1.0e-8;
pub const THICKEST_M: f64 = 1.0e-3;
/// Seconds a patch lasts on the water. The simulation's record of where she leaked fades over two hours
/// (`naval_sim::bunkers::SLICK_SECONDS`); half an hour covers an action, and the last quarter of it fades the patch out.
pub const OIL_LIFETIME: f64 = 1800.0;
/// The surface's drift, as a share of the wind, that carries the oil downwind: the wind-driven current and the waves'
/// Stokes drift together, as the gale streaks take it (`field.rs`, `STREAK_DRIFT`; the spill models' 3 % rule).
pub const DRIFT_SHARE: f64 = 0.035;
/// A leak (m³/s) under this lays nothing: the simulation publishes none under a litre a second
/// (`naval_sim::bunkers::LEAK_FLOOR_M3S`).
const LEAK_FLOOR_M3S: f64 = 5.0e-4;
/// Track metres between patches, and the longest a hull leaking without making way goes between them (s).
const SPACING: f64 = 24.0;
const INTERVAL: f64 = 4.0;
/// A patch's half-length along her track is at least this many spacings: the neighbours' films then meet within a
/// few percent of their own, where shorter ones beaded the slick (a band of another colour every patch).
const OVERLAP: f64 = 2.5;
/// The slowest her film is laid at (m/s through the water): a hull leaking where she lies pools her oil, no thicker.
const SLOWEST: f64 = 2.0;
/// Her oil's source's speed through the water is taken over this many seconds.
const SPEED_SECONDS: f64 = 0.5;
/// The most patches a slick keeps (the oldest go first): 15 km of a run at full speed.
const MOST: usize = 640;
/// The oil goes off the water (evaporates, disperses into the sea) with this time constant: the simulation's
/// (`naval_sim::bunkers::SLICK_SECONDS`).
const WEATHER_SECONDS: f64 = 7_200.0;
/// The core's oil goes into the sheen with this time constant: from a large leak it is metallic after a quarter of an
/// hour and a sheen after half.
const CORE_SECONDS: f64 = 300.0;
/// How fast the core and the sheen spread: their half-breadths grow by this many metres a √s (the length along her
/// track by half as much). The sheen is 370 m across after half an hour.
const CORE_SPREAD: f64 = 1.5;
const SHEEN_SPREAD: f64 = 4.0;
/// The sheen's film (m): a silvery sheen at its thickest, under 0.04 µm hardly to be seen but for the ripples it stills.
const SHEEN_M: f64 = 4.0e-8;
/// Burning oil: the least film heavy fuel oil lights in (Buist, Interspill 2004: Bunker C and IFO 380, about 10 mm), the
/// film a fire burns it down to (about 1 mm of residue) and how fast (Fingas, ARPEL report 28, 2007: heavy oils about
/// 1 mm a minute).
pub const IGNITE_M: f64 = 0.01;
pub const RESIDUE_M: f64 = 0.001;
pub const BURN_M_PER_S: f64 = 0.001 / 60.0;
/// The sea a fire on heavy fuel oil can hold: waves no higher than a metre (ARPEL Report 28's decision chart, Fingas
/// 2007, rules a burn out above them; waves thin a marginal slick at each crest and stop its ignition, Opstad and
/// Guénette, *Fire Safety Science* 6, 2000) and wind no stronger than some 10 m/s (the most for lighting a large burn,
/// ASTM F1990, as cited; above 15 m/s flames no longer spread upwind, ARPEL). See [`sea_holds_a_burn`].
pub const BURN_WIND_MPS: f64 = 10.0;
pub const BURN_WAVES_M: f64 = 1.0;

/// Whether a sea with this wind (m/s) and significant wave height (m) lets heavy fuel oil on it catch and keep burning.
/// In the war's record it seldom did: Barham's, Juneau's, Bismarck's, Repulse's and Prince of Wales's, Scharnhorst's,
/// Indianapolis's and Yamato's survivors swam in thick oil that did not burn; it burned where a ship still burning held
/// her fresh oil against her in calm water (Arizona in harbour for one to two days, her reports; Fuso in Surigao Strait
/// for some three hours as she went, Tully, *Battle of Surigao Strait*, 2009; Blücher as she rolled over), and at
/// Hood's site only "a flicker of flame and smoke on the water" (Nixon, Daily Express, 31 May 1941).
pub fn sea_holds_a_burn(wind_mps: f64, significant_height_m: f64) -> bool {
    wind_mps <= BURN_WIND_MPS && significant_height_m <= BURN_WAVES_M
}

/// Seconds after a slick is set alight that fresh thick oil coming up lights from what burned it (the wreck's fires,
/// the magazine's burning wreckage), whether or not any of the slick still burns.
const IGNITION_SECONDS: f64 = 120.0;
/// A burning patch lights a thick neighbour whose core reaches within this many metres of its own.
const FIRE_REACH: f64 = 10.0;
/// Where her oil goes over the side, as patches see it: this share of her length aft of her middle, along at least this
/// share of it either way and across this share of her beam: the water about her after body, where her side and her
/// screws spread what her bunkers lose.
const SOURCE_AFT: f64 = 0.3;
const SOURCE_LENGTH: f64 = 0.2;
const SOURCE_BEAM: f64 = 0.45;
/// A stamp fainter than this paints nothing (the trail's floor).
const FAINTEST: f64 = 0.015;
/// The farthest (m, each axis) the wide square may centre from the slick's newest oil, which stays inside it with a
/// margin.
const FAR_REACH: f64 = 0.4 * OIL_FAR_EXTENT;
/// Seconds between rasterisations of a slick she has stopped adding to: its patches only spread and drift.
const IDLE_INTERVAL: f64 = 1.0;
/// Her rasterisations come at this many times the trail's interval: the head of her slick stands under her after body,
/// where a step of a few metres does not show.
const HEAD_INTERVAL: f64 = 2.0;

/// Every rasterisation's version, unique across slicks, so a slot taken by another slick always repaints.
static RASTERS: AtomicU64 = AtomicU64::new(1);

/// The share of her bunkers' oil that comes up in the first minutes after she has gone (the tanks the loss ruptured;
/// the rest stays in her for years, as Arizona's still leaks a few litres a day), and how fast it comes up: an estimate.
/// Hood went down with some 4,600 t aboard and five minutes on "a huge patch of oil" lay where she went (Sunderland
/// Z/201, AIR 15/415); a tenth of it spread a millimetre thick would cover some 700 m across.
pub const WRECK_OIL_SHARE: f64 = 0.1;
pub const WRECK_OIL_SECONDS: f64 = 180.0;

/// The oil (m³/s) a wreck whose bunkers hold `bunker_m3` full puts on the water `since` seconds after she went under:
/// [`WRECK_OIL_SHARE`] of it, coming up with a time constant of [`WRECK_OIL_SECONDS`].
pub fn wreck_leak_m3s(bunker_m3: f64, since: f64) -> f64 {
    if !(since >= 0.0 && bunker_m3 > 0.0) {
        return 0.0;
    }
    WRECK_OIL_SHARE * bunker_m3 / WRECK_OIL_SECONDS * (-since / WRECK_OIL_SECONDS).exp()
}

/// A film's thickness (m) on the atlas's scale: 0 at [`THINNEST_M`] and below, 1 at [`THICKEST_M`] and above, by its
/// logarithm, so a byte holds it to 5 % from a sheen to the oil's own colour.
pub fn encode(thickness: f64) -> f64 {
    if thickness.is_nan() || thickness <= THINNEST_M {
        return 0.0;
    }
    ((thickness / THINNEST_M).ln() / (THICKEST_M / THINNEST_M).ln()).min(1.0)
}

/// The thickness (m) an [`encode`]d value stands for.
pub fn decode(encoded: f64) -> f64 {
    THINNEST_M * (THICKEST_M / THINNEST_M).powf(encoded.clamp(0.0, 1.0))
}

fn smooth(value: f64) -> f64 {
    let t = value.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One patch of oil: where it lies (it drifts), when it went over the side, across her track then, its first film (m)
/// and its first half-breadth and half-length (m).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Patch {
    x: f64,
    z: f64,
    born: f64,
    right_x: f64,
    right_z: f64,
    thickness: f64,
    width: f64,
    length: f64,
    /// The film burned off its core (m), and whether it burns now.
    burned: f64,
    burning: bool,
}

impl Patch {
    /// Its core and its sheen at `age` seconds: each a half-breadth, a half-length and the film's thickness (m) at its
    /// middle. The oil thins as it spreads across her track (along it her other patches spread into it as it spreads
    /// into theirs) and weathers; the core's goes into the sheen.
    fn films(&self, age: f64) -> [(f64, f64, f64); 2] {
        let age = age.max(0.0);
        let root = age.sqrt();
        let left = (-age / WEATHER_SECONDS).exp();
        let core = (
            self.width + CORE_SPREAD * root,
            self.length + 0.5 * CORE_SPREAD * root,
        );
        let sheen = (
            self.width + SHEEN_SPREAD * root,
            self.length + 0.5 * SHEEN_SPREAD * root,
        );
        let core_film = (self.thickness * self.width / core.0 * left * (-age / CORE_SECONDS).exp()
            - self.burned)
            .max(0.0);
        // The sheen can hold no more oil than the patch has.
        let sheen_film = SHEEN_M.min(self.thickness * self.width / sheen.0 * left);
        [(core.0, core.1, core_film), (sheen.0, sheen.1, sheen_film)]
    }
}

/// Where her oil goes over the side, world x and z: [`SOURCE_AFT`] of her length aft of her middle.
fn source_point(motion: Motion, hull: &WakeHull) -> [f64; 2] {
    let (forward_x, forward_z) = (motion.heading.sin(), -motion.heading.cos());
    let (right_x, right_z) = (-forward_z, forward_x);
    let aft = hull.center_z + hull.length * SOURCE_AFT;
    [
        motion.x + right_x * hull.center_x - forward_x * aft,
        motion.z + right_z * hull.center_x - forward_z * aft,
    ]
}

/// One hull's oil on the water: the patches she has laid, and the stamps of their last rasterisation.
#[derive(Clone, Debug)]
pub struct OilSlick {
    patches: VecDeque<Patch>,
    /// Her last pose and where her oil went over the side then, while she leaked.
    previous: Option<(Motion, [f64; 2])>,
    /// The oil going over the side where she is now, painted at every rasterisation so the slick keeps up with her.
    head: Option<Patch>,
    /// Where the last rasterisation framed the squares: the slick's newest oil ([`OilSlick::anchor`]).
    anchor: [f64; 2],
    /// How fast her oil's source moves through the water (m/s, smoothed over [`SPEED_SECONDS`]).
    speed: f64,
    /// Track metres and seconds since her last patch.
    since: f64,
    idle: f64,
    time: f64,
    elapsed: f64,
    dirty: bool,
    pub collector: StampCollector,
    /// What the last rasterisation painted in each channel (minX, minZ, maxX, maxZ), padded by a texel.
    pub painted: [[f64; 4]; 2],
    /// Moves with every rasterisation, unique across slicks.
    pub raster_version: u64,
    /// Set alight: where, how far about it (m) and until when fresh thick oil lights from it ([`IGNITION_SECONDS`]).
    ignition: Option<([f64; 2], f64, f64)>,
}

/// A burning patch of a slick: where, across which way, its core's half-breadth and half-length (m) and the film still
/// on it (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OilFire {
    pub x: f64,
    pub z: f64,
    pub right: [f64; 2],
    pub width: f64,
    pub length: f64,
    pub film: f64,
}

impl Default for OilSlick {
    fn default() -> Self {
        Self::new()
    }
}

impl OilSlick {
    pub fn new() -> Self {
        Self {
            patches: VecDeque::new(),
            previous: None,
            head: None,
            anchor: [0.0; 2],
            speed: 0.0,
            since: 0.0,
            idle: 0.0,
            time: 0.0,
            elapsed: 0.0,
            dirty: false,
            collector: StampCollector::with_extents([OIL_NEAR_EXTENT, OIL_FAR_EXTENT]),
            painted: [EMPTY; 2],
            raster_version: 0,
            ignition: None,
        }
    }

    /// Sets her oil alight within `reach` m of `at` (world x, z): her own fire going down with her.
    /// Thick oil there burns now, and thick oil coming up there for [`IGNITION_SECONDS`] after.
    pub fn ignite(&mut self, at: [f64; 2], reach: f64) {
        self.ignition = Some((at, reach.max(0.0), self.time + IGNITION_SECONDS));
        self.spread_fire();
    }

    /// Puts out all of it that burns, and what would still have lit: the sea has got up past what holds a burn
    /// ([`sea_holds_a_burn`]; its waves break a burning slick up).
    pub fn put_out(&mut self) {
        self.ignition = None;
        for p in &mut self.patches {
            p.burning = false;
        }
    }

    /// Whether any of it burns.
    pub fn burning(&self) -> bool {
        self.patches.iter().any(|p| p.burning)
    }

    /// The patches burning now.
    pub fn fires(&self) -> impl Iterator<Item = OilFire> + '_ {
        let now = self.time;
        self.patches.iter().filter(|p| p.burning).map(move |p| {
            let [(width, length, film), _] = p.films(now - p.born);
            OilFire {
                x: p.x,
                z: p.z,
                right: [p.right_x, p.right_z],
                width,
                length,
                film,
            }
        })
    }

    /// The fire takes hold of thick oil: where it was set alight (while that lasts) and beside what burns.
    fn spread_fire(&mut self) {
        let now = self.time;
        let lighting = self
            .ignition
            .filter(|(_, _, until)| now <= *until)
            .map(|(at, reach, _)| (at, reach));
        let burning: Vec<(f64, f64, f64)> = self
            .patches
            .iter()
            .filter(|p| p.burning)
            .map(|p| (p.x, p.z, p.films(now - p.born)[0].0))
            .collect();
        if lighting.is_none() && burning.is_empty() {
            return;
        }
        for p in &mut self.patches {
            if p.burning {
                continue;
            }
            let [(width, _, film), _] = p.films(now - p.born);
            if film < IGNITE_M {
                continue;
            }
            let near =
                |x: f64, z: f64, reach: f64| (p.x - x).hypot(p.z - z) <= width + reach + FIRE_REACH;
            if lighting.is_some_and(|([x, z], reach)| near(x, z, reach))
                || burning.iter().any(|&(x, z, reach)| near(x, z, reach))
            {
                p.burning = true;
                self.dirty = true;
            }
        }
    }

    /// Burns each burning patch down by `dt` seconds' worth, out at its residue.
    fn burn(&mut self, dt: f64) {
        let now = self.time;
        for p in &mut self.patches {
            if !p.burning {
                continue;
            }
            p.burned += BURN_M_PER_S * dt;
            if p.films(now - p.born)[0].2 <= RESIDUE_M {
                p.burning = false;
            }
            self.dirty = true;
        }
    }

    /// Whether the last rasterisation painted any oil: the slick takes a tile of the atlas.
    pub fn drawn(&self) -> bool {
        self.painted[1] != EMPTY || self.painted[0] != EMPTY
    }

    /// Patches on the water now.
    pub fn patches(&self) -> usize {
        self.patches.len()
    }

    /// Centres of the fine and the wide square at the last rasterisation.
    pub fn centers(&self) -> [[f64; 2]; 2] {
        [self.collector.center, self.collector.slick_center]
    }

    /// Advance by `dt` (0 does nothing): every patch drifts with the surface's `drift` (m/s, world x and z) and ages,
    /// she lays new ones along her path while she leaks `leak_m3s`, and the slick is rasterised at most every
    /// `interval` seconds (the trail's: more often near the camera).
    pub fn update(
        &mut self,
        motion: Motion,
        hull: &WakeHull,
        leak_m3s: f64,
        drift: [f64; 2],
        dt: f64,
        interval: f64,
    ) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.time += dt;
        self.elapsed += dt;
        let now = self.time;
        if drift.iter().all(|v| v.is_finite()) && drift != [0.0; 2] {
            for patch in &mut self.patches {
                patch.x += drift[0] * dt;
                patch.z += drift[1] * dt;
            }
        }
        let before = self.patches.len();
        while self
            .patches
            .front()
            .is_some_and(|p| now - p.born > OIL_LIFETIME)
        {
            self.patches.pop_front();
        }
        if self.patches.len() != before {
            self.dirty = true;
        }
        self.burn(dt);
        let leaking = leak_m3s.is_finite() && leak_m3s >= LEAK_FLOOR_M3S;
        let source = source_point(motion, hull);
        match self.previous.filter(|_| leaking) {
            Some((last, from)) => {
                let travel = (source[0] - from[0]).hypot(source[1] - from[1]);
                // Her oil's source sweeps no farther than her stern can (the trail's bound on a jump).
                let turn = {
                    let d = motion.heading - last.heading;
                    d.sin().atan2(d.cos()).abs()
                };
                let expected =
                    last.speed.abs().max(motion.speed.abs()) * dt * 3.0 + hull.length * turn;
                // More track in one step than a slick keeps patches for is no step either (a pose gone wild).
                if travel.is_nan() || travel > 100.0 + expected || travel > SPACING * MOST as f64 {
                    // She was moved, not sailed there: her oil stays where it lies and her slick starts again.
                    self.restart(motion);
                } else {
                    let k = 1.0 - (-dt / SPEED_SECONDS).exp();
                    self.speed += (travel / dt - self.speed) * k;
                    self.lay(last, from, motion, source, travel, leak_m3s, dt, hull);
                }
            }
            None => self.restart(motion),
        }
        self.spread_fire();
        self.head = None;
        if leaking {
            // Her film where she is now, as her next patch will lay it.
            let film = self.film(leak_m3s, hull);
            self.head = Some(self.patch(motion, source, now, film, hull));
            self.previous = Some((motion, source));
        } else {
            self.previous = None;
        }
        let drawn = !self.patches.is_empty() || self.head.is_some();
        let interval = if self.head.is_some() {
            interval * HEAD_INTERVAL
        } else {
            interval.max(IDLE_INTERVAL)
        };
        if self.elapsed < interval || (!drawn && !self.dirty) {
            return;
        }
        self.elapsed = if self.elapsed.is_finite() {
            self.elapsed % interval
        } else {
            0.0
        };
        self.rasterize(self.anchor());
        self.dirty = drawn;
    }

    /// Where the squares are framed: the slick's newest oil, her head while she leaks (where her hull is), else the last
    /// patch she laid, so a slick she has left stays drawn where it lies however far she goes; where they were last
    /// framed when there is none.
    fn anchor(&self) -> [f64; 2] {
        self.head
            .as_ref()
            .or(self.patches.back())
            .map_or(self.anchor, |p| [p.x, p.z])
    }

    /// The film (m) she lays leaking `leak_m3s`: her oil per area of the track her oil's source sweeps, over the
    /// breadth it lies across; at [`SLOWEST`] when she barely makes way.
    fn film(&self, leak_m3s: f64, hull: &WakeHull) -> f64 {
        let (width, _) = Self::footprint(hull);
        leak_m3s / (2.0 * width * self.speed.max(SLOWEST))
    }

    /// A patch's first half-breadth and half-length (m).
    fn footprint(hull: &WakeHull) -> (f64, f64) {
        (
            (hull.beam * SOURCE_BEAM).max(1.0),
            (hull.length * SOURCE_LENGTH).max(SPACING * OVERLAP),
        )
    }

    fn patch(
        &self,
        motion: Motion,
        at: [f64; 2],
        born: f64,
        thickness: f64,
        hull: &WakeHull,
    ) -> Patch {
        let (width, length) = Self::footprint(hull);
        Patch {
            x: at[0],
            z: at[1],
            born,
            right_x: motion.heading.cos(),
            right_z: motion.heading.sin(),
            thickness,
            width,
            length,
            burned: 0.0,
            burning: false,
        }
    }

    /// Lay the patches of one step, her oil's source moving `travel` metres from `from` to `to` while she went from
    /// `last` to `motion`: one every [`SPACING`] metres of its path and, when she barely makes way, one where she is
    /// every [`INTERVAL`] seconds.
    #[allow(clippy::too_many_arguments)]
    fn lay(
        &mut self,
        last: Motion,
        from: [f64; 2],
        motion: Motion,
        to: [f64; 2],
        travel: f64,
        leak_m3s: f64,
        dt: f64,
        hull: &WakeHull,
    ) {
        let start = self.time - dt;
        let turn = {
            let d = motion.heading - last.heading;
            d.sin().atan2(d.cos())
        };
        let film = self.film(leak_m3s, hull);
        let mut taken = 0.0;
        if travel > 1e-6 {
            let mut along = SPACING - self.since;
            while along <= travel {
                let fraction = along / travel;
                let pose = Motion {
                    x: last.x + (motion.x - last.x) * fraction,
                    z: last.z + (motion.z - last.z) * fraction,
                    heading: last.heading + turn * fraction,
                    speed: last.speed + (motion.speed - last.speed) * fraction,
                };
                let at = [
                    from[0] + (to[0] - from[0]) * fraction,
                    from[1] + (to[1] - from[1]) * fraction,
                ];
                let patch = self.patch(pose, at, start + dt * fraction, film, hull);
                self.push(patch);
                taken = fraction;
                along += SPACING;
            }
            self.since = (self.since + travel) % SPACING;
        }
        self.idle = if taken > 0.0 {
            dt * (1.0 - taken)
        } else {
            self.idle + dt
        };
        if self.idle >= INTERVAL {
            let patch = self.patch(motion, to, self.time, film, hull);
            self.push(patch);
            self.idle = 0.0;
            self.since = 0.0;
        }
    }

    fn push(&mut self, patch: Patch) {
        if patch.thickness.is_nan()
            || patch.thickness <= 0.0
            || !patch.x.is_finite()
            || !patch.z.is_finite()
        {
            return;
        }
        self.patches.push_back(patch);
        while self.patches.len() > MOST {
            self.patches.pop_front();
        }
        self.dirty = true;
    }

    /// Her slick starts again from the next step, `motion` her pose now: what lies on the water stays.
    fn restart(&mut self, motion: Motion) {
        self.since = 0.0;
        self.idle = 0.0;
        // Her speed through the water as her helm shows it, until her oil's source's own track says.
        self.speed = if motion.speed.is_finite() {
            motion.speed.abs()
        } else {
            0.0
        };
    }

    /// Moves the slick whole by `offset` (a theatre's re-centre) and paints it there.
    pub fn translate(&mut self, offset: [f64; 2]) {
        let shift = |p: &mut Patch| {
            p.x += offset[0];
            p.z += offset[1];
        };
        self.patches.iter_mut().for_each(shift);
        self.head.iter_mut().for_each(shift);
        if let Some((motion, source)) = &mut self.previous {
            motion.x += offset[0];
            motion.z += offset[1];
            source[0] += offset[0];
            source[1] += offset[1];
        }
        self.anchor = [self.anchor[0] + offset[0], self.anchor[1] + offset[1]];
        if let Some((at, ..)) = &mut self.ignition {
            *at = [at[0] + offset[0], at[1] + offset[1]];
        }
        self.repaint();
    }

    /// Carries the slick into a scene laid on another chart (a theatre's re-lay), as the trail carries hers: each
    /// patch's place through the relay, its breadth's direction through the charts' Jacobians, her last heading
    /// through the turn between the two grids.
    pub fn relay(&mut self, relay: &dyn naval_render::relay::SceneRelay) {
        let carry = |p: &mut Patch| {
            let at = [p.x, 0.0, p.z];
            [p.right_x, p.right_z] = relay.direction(at, [p.right_x, p.right_z]);
            [p.x, p.z] = relay.point2([p.x, p.z]);
        };
        self.patches.iter_mut().for_each(carry);
        self.head.iter_mut().for_each(carry);
        if let Some((motion, source)) = &mut self.previous {
            let at = [motion.x, 0.0, motion.z];
            motion.heading = relay.heading(at, motion.heading);
            [motion.x, motion.z] = relay.point2([motion.x, motion.z]);
            *source = relay.point2(*source);
        }
        self.anchor = relay.point2(self.anchor);
        if let Some((at, ..)) = &mut self.ignition {
            *at = relay.point2(*at);
        }
        self.repaint();
    }

    /// Paints the slick again where it now lies (after a move), if there is any.
    fn repaint(&mut self) {
        if self.drawn() || !self.patches.is_empty() || self.head.is_some() {
            self.rasterize(self.anchor());
        }
    }

    /// Lay this moment's stamps: each patch's core and sheen in the wide square, and in the fine one those that reach
    /// it, the squares framed on `anchor` (world x, z: [`OilSlick::anchor`]).
    fn rasterize(&mut self, anchor: [f64; 2]) {
        self.anchor = anchor;
        let near_cell = OIL_NEAR_EXTENT / f64::from(OIL_RESOLUTION);
        let near = [
            (anchor[0] / near_cell).round() * near_cell,
            (anchor[1] / near_cell).round() * near_cell,
        ];
        let far = self.frame_far(anchor);
        self.collector.begin(near, far);
        self.painted = [EMPTY; 2];
        let now = self.time;
        let patches = std::mem::take(&mut self.patches);
        let head = self.head;
        for patch in patches.iter().chain(head.iter()) {
            let age = now - patch.born;
            // The last quarter of its life fades it out.
            let fade = 1.0 - smooth((age - OIL_LIFETIME * 0.75) / (OIL_LIFETIME * 0.25));
            for (width, length, film) in patch.films(age) {
                let strength = encode(film) * fade;
                if strength < FAINTEST {
                    continue;
                }
                for channel in [1, 0] {
                    self.stamp(patch, width, length, strength, channel);
                }
            }
        }
        self.patches = patches;
        self.raster_version = RASTERS.fetch_add(1, Ordering::Relaxed);
    }

    /// Centre the wide square on the slick it holds, keeping its newest oil at `anchor` inside it with a margin: the
    /// oldest goes past its edge first.
    fn frame_far(&self, anchor: [f64; 2]) -> [f64; 2] {
        let cell = OIL_FAR_EXTENT / f64::from(OIL_RESOLUTION);
        let (mut min_x, mut max_x, mut min_z, mut max_z) =
            (anchor[0], anchor[0], anchor[1], anchor[1]);
        for patch in &self.patches {
            if (patch.x - anchor[0]).abs() > FAR_REACH * 2.0
                || (patch.z - anchor[1]).abs() > FAR_REACH * 2.0
            {
                continue;
            }
            min_x = min_x.min(patch.x);
            max_x = max_x.max(patch.x);
            min_z = min_z.min(patch.z);
            max_z = max_z.max(patch.z);
        }
        let x = anchor[0] + ((min_x + max_x) / 2.0 - anchor[0]).clamp(-FAR_REACH, FAR_REACH);
        let z = anchor[1] + ((min_z + max_z) / 2.0 - anchor[1]).clamp(-FAR_REACH, FAR_REACH);
        [(x / cell).round() * cell, (z / cell).round() * cell]
    }

    /// One film of `patch` into `channel` (0 the fine square, 1 the wide one), when it reaches that square.
    fn stamp(&mut self, patch: &Patch, width: f64, length: f64, strength: f64, channel: u8) {
        let (center, extent) = self.collector.frame(channel);
        let pad = extent / f64::from(OIL_RESOLUTION);
        let (rx, rz) = (patch.right_x, patch.right_z);
        let reach_x = rx.abs() * width + rz.abs() * length + pad;
        let reach_z = rz.abs() * width + rx.abs() * length + pad;
        let half = extent / 2.0;
        if (patch.x - center[0]).abs() > half + reach_x
            || (patch.z - center[1]).abs() > half + reach_z
        {
            return;
        }
        let b = &mut self.painted[channel as usize];
        *b = [
            b[0].min(patch.x - reach_x).max(center[0] - half),
            b[1].min(patch.z - reach_z).max(center[1] - half),
            b[2].max(patch.x + reach_x).min(center[0] + half),
            b[3].max(patch.z + reach_z).min(center[1] + half),
        ];
        self.collector.stamps.push(Stamp {
            x: patch.x,
            z: patch.z,
            right_x: rx,
            right_z: rz,
            width,
            length,
            strength,
            ring: false,
            channel,
            across: NO_BAND,
            start: NO_START,
        });
        self.collector.version += 1;
    }
}

/// One oil slot's rows for the surface (`shaders/oil.wgsl`): the fine square's centre and the slot's atlas tile, the
/// wide square's centre, and what each painted and both together (minX, minZ, maxX, maxZ).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OilRows {
    pub near: [f32; 4],
    pub far: [f32; 4],
    pub near_box: [f32; 4],
    pub far_box: [f32; 4],
    pub box_: [f32; 4],
}

impl OilRows {
    /// The rows of `slick` in atlas slot `slot`.
    pub fn of(slick: &OilSlick, slot: usize) -> Self {
        let (tx, ty) = ((slot % OIL_TILES) as f32, (slot / OIL_TILES) as f32);
        let [near, far] = slick.centers();
        let [near_box, far_box] = slick.painted;
        let both = [
            near_box[0].min(far_box[0]),
            near_box[1].min(far_box[1]),
            near_box[2].max(far_box[2]),
            near_box[3].max(far_box[3]),
        ];
        let f = |b: [f64; 4]| b.map(|v| v as f32);
        Self {
            near: [near[0] as f32, near[1] as f32, tx, ty],
            far: [far[0] as f32, far[1] as f32, 0.0, 0.0],
            near_box: f(near_box),
            far_box: f(far_box),
            box_: f(both),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_render::relay::SceneRelay as _;

    fn bismarck() -> WakeHull {
        WakeHull {
            length: 250.0,
            beam: 36.0,
            forward_speed: 15.4,
            center_x: 0.0,
            center_z: 0.0,
        }
    }

    /// Her oil's source at heading 0 (toward −z): 75 m aft of her middle, toward +z.
    const AFT: f64 = 75.0;

    /// `seconds` of a straight run north (heading 0, toward −z) from `from` at `speed` m/s through the water, leaking
    /// `leak`, in steps of `dt`, the surface drifting at `drift`.
    /// Returns where she ends.
    fn run(
        slick: &mut OilSlick,
        seconds: f64,
        speed: f64,
        leak: f64,
        drift: [f64; 2],
        dt: f64,
        from: f64,
    ) -> f64 {
        let steps = (seconds / dt).round() as usize;
        let mut z = from;
        for _ in 0..steps {
            z -= speed * dt;
            slick.update(
                Motion {
                    x: 0.0,
                    z,
                    heading: 0.0,
                    speed,
                },
                &bismarck(),
                leak,
                drift,
                dt,
                0.05,
            );
        }
        z
    }

    /// The film (m) the stamps of `channel` paint at a world point, as `stamps.wgsl` paints them (before the texels):
    /// the thickest one there.
    fn film_at(slick: &OilSlick, channel: u8, x: f64, z: f64) -> f64 {
        let encoded = slick
            .collector
            .stamps
            .iter()
            .filter(|s| s.channel == channel)
            .map(|s| {
                let (dx, dz) = (x - s.x, z - s.z);
                let across = (dx * s.right_x + dz * s.right_z) / s.width;
                let along = (-dx * s.right_z + dz * s.right_x) / s.length;
                let radius = across * across + along * along;
                if radius < 1.0 {
                    (1.0 - smooth(radius)) * s.strength
                } else {
                    0.0
                }
            })
            .fold(0.0, f64::max);
        if encoded > 0.0 { decode(encoded) } else { 0.0 }
    }

    /// Where Bismarck went down, with some 7,000 m³ in her bunkers: her oil comes up where she went over minutes and
    /// spreads; set alight, the thick of it burns, each patch down at a millimetre a minute to its residue, the fire
    /// taking the thick oil still coming up, and is out within half an hour.
    #[test]
    fn a_wrecks_oil_comes_up_where_she_went_and_burns_where_it_lies_thick() {
        let bunker = 7_000.0;
        // A tenth of it, over minutes.
        let total: f64 = (0..7200)
            .map(|s| wreck_leak_m3s(bunker, f64::from(s) * 0.5) * 0.5)
            .sum();
        assert!(
            (total / (WRECK_OIL_SHARE * bunker) - 1.0).abs() < 0.01,
            "{total:.0} m³"
        );
        assert_eq!(wreck_leak_m3s(bunker, -1.0), 0.0);
        assert_eq!(wreck_leak_m3s(0.0, 10.0), 0.0);
        let mut slick = OilSlick::new();
        let wreck = Motion {
            x: 500.0,
            z: -200.0,
            heading: 0.3,
            speed: 0.0,
        };
        let (dt, mut t) = (0.25, 0.0);
        let (mut burning_most, mut out_at) = (0, None);
        while t < 1800.0 {
            slick.update(
                wreck,
                &bismarck(),
                wreck_leak_m3s(bunker, t),
                [0.1, 0.0],
                dt,
                0.05,
            );
            t += dt;
            if (t - 5.0).abs() < dt / 2.0 {
                // Her own fire going down with her, over her length.
                slick.ignite([500.0, -200.0], 125.0);
            }
            let fires = slick.fires().count();
            burning_most = burning_most.max(fires);
            if t > 10.0 && fires == 0 && out_at.is_none() {
                out_at = Some(t);
            }
            if (t - 60.0).abs() < dt / 2.0 {
                // A minute on: thick oil where she went, burning, the film there a few centimetres.
                let thickest = slick.fires().map(|f| f.film).fold(0.0, f64::max);
                assert!(thickest > IGNITE_M, "{thickest} m");
                assert!(
                    slick
                        .fires()
                        .all(|f| (f.x - 500.0).hypot(f.z + 200.0) < 150.0)
                );
            }
        }
        assert!(slick.patches() > 200, "{} patches", slick.patches());
        assert!(burning_most > 5, "{burning_most} burning");
        let out = out_at.expect("the fire went out");
        assert!((300.0..1800.0).contains(&out), "out at {out:.0} s");
        // What burned is down to its residue, no more.
        let now = slick.time;
        for p in slick.patches.iter().filter(|p| p.burned > 0.0) {
            assert!(p.films(now - p.born)[0].2 <= RESIDUE_M + 1e-9);
        }
        // Unlit, it never burns.
        let mut unlit = OilSlick::new();
        for _ in 0..1200 {
            unlit.update(
                wreck,
                &bismarck(),
                wreck_leak_m3s(bunker, 0.0),
                [0.0; 2],
                dt,
                0.05,
            );
        }
        assert!(!unlit.burning());
    }

    /// Heavy fuel oil burns on a calm sea, not in a seaway: the staged battles' 9 m/s sea (1.8 m) holds no burn.
    #[test]
    fn only_a_calm_sea_holds_a_burn() {
        assert!(sea_holds_a_burn(3.0, 0.5));
        assert!(!sea_holds_a_burn(9.0, 1.8));
        assert!(!sea_holds_a_burn(12.0, 0.8));
        assert!(!sea_holds_a_burn(4.0, 1.4));
        // Lit, then the sea gets up: it goes out, and the oil coming up after does not light.
        let (wreck, bunker, dt) = (
            Motion {
                x: 0.0,
                z: 0.0,
                heading: 0.0,
                speed: 0.0,
            },
            7_000.0,
            0.5,
        );
        let mut slick = OilSlick::new();
        let step = |slick: &mut OilSlick, t: f64| {
            slick.update(
                wreck,
                &bismarck(),
                wreck_leak_m3s(bunker, t),
                [0.1, 0.0],
                dt,
                0.05,
            );
        };
        for k in 0..60 {
            step(&mut slick, f64::from(k) * dt);
        }
        slick.ignite([0.0, 0.0], 125.0);
        step(&mut slick, 30.0);
        assert!(slick.burning());
        slick.put_out();
        for k in 0..60 {
            step(&mut slick, 30.5 + f64::from(k) * dt);
            assert!(!slick.burning());
        }
    }

    #[test]
    fn the_scale_holds_a_film_to_a_few_percent() {
        for thickness in [2e-8, 1e-7, 1e-6, 5e-5, 9e-4] {
            let back = decode(encode(thickness));
            assert!((back / thickness - 1.0).abs() < 1e-9, "{thickness} {back}");
            // A byte of it: within 5 % of the film.
            let byte = (encode(thickness) * 255.0).round() / 255.0;
            assert!((decode(byte) / thickness - 1.0).abs() < 0.05, "{thickness}");
        }
        assert_eq!(encode(0.0), 0.0);
        assert_eq!(encode(f64::NAN), 0.0);
        assert_eq!(encode(1.0), 1.0);
    }

    #[test]
    fn the_shader_reads_the_atlas_on_this_scale() {
        let wgsl = include_str!("../shaders/oil.wgsl");
        for line in [
            format!("const OIL_TILES: f32 = {:.1};", OIL_TILES as f64),
            format!("const OIL_SLOTS: i32 = {OIL_SLOTS};"),
            "const OIL_THINNEST: f32 = 1e-8;".to_owned(),
            format!(
                "const OIL_RANGE: f32 = {:.6};",
                (THICKEST_M / THINNEST_M).ln()
            ),
        ] {
            assert!(wgsl.contains(&line), "shaders/oil.wgsl lacks `{line}`");
        }
    }

    /// No leak lays nothing; a leak lays its film along her track by its rate, from her after body astern.
    #[test]
    fn a_leak_lays_a_slick_along_her_track_by_its_rate() {
        let mut dry = OilSlick::new();
        run(&mut dry, 60.0, 21.0, 0.0, [0.0; 2], 0.1, 0.0);
        assert_eq!(dry.patches(), 0);
        assert!(!dry.drawn() && dry.collector.stamps.is_empty());

        let mut heavy = OilSlick::new();
        let mut light = OilSlick::new();
        let z = run(&mut heavy, 60.0, 21.0, 0.3, [0.0; 2], 0.1, 0.0);
        run(&mut light, 60.0, 21.0, 0.003, [0.0; 2], 0.1, 0.0);
        // One every 24 m of her 1,260 m.
        assert!(
            (50..=54).contains(&heavy.patches()),
            "{} patches",
            heavy.patches()
        );
        assert!(heavy.drawn() && light.drawn());
        let source = z + AFT;
        // 0.3 m³/s at 21 m/s over 16 m either side is 0.45 mm laid, thinned by its spreading in the quarter minute since.
        let thick = film_at(&heavy, 0, 0.0, source + 300.0);
        let thin = film_at(&light, 0, 0.0, source + 300.0);
        assert!(thick > 2e-4 && thick < 4.5e-4, "{thick}");
        assert!(
            (thick / thin - 100.0).abs() < 10.0,
            "{thick} against {thin}"
        );
        // Evenly along her track: patches 24 m apart meet within a few percent.
        let films: Vec<f64> = (0..48)
            .map(|i| film_at(&heavy, 0, 0.0, source + 400.0 + f64::from(i)))
            .collect();
        let (low, high) = films
            .iter()
            .fold((f64::MAX, 0.0_f64), |(l, h), &f| (l.min(f), h.max(f)));
        assert!(high / low < 1.15, "beaded from {low} to {high}");
        // None ahead of her (her bow is 125 m ahead of her middle), oil at her after body, none a kilometre abeam.
        assert_eq!(film_at(&heavy, 0, 0.0, z - 200.0), 0.0);
        assert!(film_at(&heavy, 0, 0.0, source) > 1e-4);
        assert_eq!(film_at(&heavy, 1, 1000.0, source + 300.0), 0.0);
        // Both squares hold it; the fine one is centred on her newest oil, at her after body.
        assert!(film_at(&heavy, 1, 0.0, source + 300.0) > 1e-4);
        let [near, _] = heavy.centers();
        assert!((near[1] - source).abs() < 5.0 && near[0].abs() < 5.0);
    }

    /// A patch spreads wider and thins as it ages, its core going into the sheen; at the end of its life it is gone.
    #[test]
    fn the_oil_spreads_thins_and_goes() {
        let mut slick = OilSlick::new();
        let z = run(&mut slick, 30.0, 21.0, 0.3, [0.0; 2], 0.1, 0.0);
        // The oil laid 20 s in, 10 s ago.
        let middle = 20.0 * -21.0 + AFT;
        let breadth = |slick: &OilSlick| {
            (0..600)
                .map(f64::from)
                .find(|x| film_at(slick, 1, *x, middle) == 0.0)
                .unwrap()
        };
        let young = film_at(&slick, 1, 0.0, middle);
        let young_breadth = breadth(&slick);
        // She stops leaking and sails on (6 km in five minutes, 31 km by the end: the slick stays where it lies, drawn);
        // five minutes later the oil she laid is wider and thinner.
        let z = run(&mut slick, 300.0, 21.0, 0.0, [0.0; 2], 0.1, z);
        let old = film_at(&slick, 1, 0.0, middle);
        let old_breadth = breadth(&slick);
        assert!(old < young / 4.0, "{young} then {old}");
        assert!(
            old_breadth > young_breadth * 1.8,
            "{young_breadth} m then {old_breadth} m"
        );
        // Twenty minutes on, under a micron (a rainbow going to a sheen) and wider still; by the end of its life, gone.
        let z = run(&mut slick, 1200.0, 21.0, 0.0, [0.0; 2], 0.5, z);
        let late = film_at(&slick, 1, 0.0, middle);
        assert!(late > 1e-8 && late < 1e-6, "{late}");
        assert!(breadth(&slick) > old_breadth);
        run(&mut slick, 400.0, 21.0, 0.0, [0.0; 2], 0.5, z);
        assert_eq!(slick.patches(), 0);
        assert!(!slick.drawn(), "a slick past its life paints nothing");
    }

    /// A light leak's core goes into its sheen within twenty minutes: a film a few hundredths of a micron thick, far wider
    /// than she is.
    #[test]
    fn a_light_leak_thins_to_a_sheen() {
        let mut slick = OilSlick::new();
        let z = run(&mut slick, 30.0, 21.0, 0.003, [0.0; 2], 0.1, 0.0);
        run(&mut slick, 1080.0, 21.0, 0.0, [0.0; 2], 0.5, z);
        let middle = 20.0 * -21.0 + AFT;
        let film = film_at(&slick, 1, 0.0, middle);
        assert!(film > 3e-8 && film < 5e-8, "{film}");
        assert!(film_at(&slick, 1, 120.0, middle) > 0.0);
    }

    /// The slick drifts downwind with the surface; her track does not.
    #[test]
    fn the_slick_drifts_downwind() {
        let mut slick = OilSlick::new();
        // 3.5 % of a 9 m/s wind toward +x.
        let drift = [DRIFT_SHARE * 9.0, 0.0];
        let z = run(&mut slick, 20.0, 21.0, 0.3, drift, 0.1, 0.0);
        run(&mut slick, 200.0, 0.0, 0.0, drift, 0.1, z);
        // The patch laid 10 s in lies about 0.315 m/s × 210 s = 66 m downwind of her track.
        let middle = 10.0 * -21.0 + AFT;
        let crest = (-100..200)
            .map(|x| (f64::from(x), film_at(&slick, 1, f64::from(x), middle)))
            .fold(
                (0.0, 0.0),
                |best, (x, film)| if film > best.1 { (x, film) } else { best },
            );
        assert!((crest.0 - 66.0).abs() < 12.0, "thickest at x {}", crest.0);
    }

    /// A hull leaking without making way lays her oil about her after body every few seconds: it pools there.
    #[test]
    fn a_hull_lying_stopped_pools_her_oil() {
        let mut slick = OilSlick::new();
        run(&mut slick, 60.0, 0.0, 0.05, [0.0; 2], 0.1, 0.0);
        assert!(
            (14..=16).contains(&slick.patches()),
            "{} patches",
            slick.patches()
        );
        // 0.05 m³/s laid as at 2 m/s over 16 m either side.
        let pool = film_at(&slick, 0, 0.0, AFT);
        assert!(pool > 5e-4 && pool < 1e-3, "{pool}");
        // Along her after body, thinning away from her source: a metallic film 40 m forward of it, none past her
        // middle.
        let along = film_at(&slick, 0, 0.0, AFT - 40.0);
        assert!(along > 1e-6 && along < pool / 10.0, "{along}");
        assert_eq!(film_at(&slick, 0, 0.0, -10.0), 0.0);
    }

    /// The head keeps up with her: the oil reaches her source at every rasterisation, between patches as at them.
    #[test]
    fn the_slick_keeps_up_with_her() {
        let mut slick = OilSlick::new();
        let (speed, dt) = (21.0, 1.0 / 60.0);
        let mut z = 0.0;
        let mut weakest = f64::MAX;
        let mut version = 0;
        for frame in 0..1200 {
            z -= speed * dt;
            slick.update(
                Motion {
                    x: 0.0,
                    z,
                    heading: 0.0,
                    speed,
                },
                &bismarck(),
                0.3,
                [0.0; 2],
                dt,
                0.05,
            );
            if frame > 600 && slick.raster_version != version {
                version = slick.raster_version;
                // Just rasterised: the film at her source and 20 m astern of it.
                for behind in [0.0, 20.0] {
                    weakest = weakest.min(film_at(&slick, 0, 0.0, z + AFT + behind));
                }
            }
        }
        assert!(
            weakest > 2e-4,
            "the oil at her after body fell to {weakest}"
        );
    }

    /// A jump (a hull moved, not sailed) starts her slick again where she is and leaves what she laid where it lies.
    #[test]
    fn a_jump_leaves_the_oil_where_it_lies() {
        let mut slick = OilSlick::new();
        let z = run(&mut slick, 20.0, 21.0, 0.3, [0.0; 2], 0.1, 0.0);
        let before = slick.patches();
        let laid = film_at(&slick, 1, 0.0, z + 300.0);
        // Five kilometres east, at once.
        slick.update(
            Motion {
                x: 5000.0,
                z,
                heading: 0.0,
                speed: 21.0,
            },
            &bismarck(),
            0.3,
            [0.0; 2],
            0.1,
            0.05,
        );
        assert_eq!(slick.patches(), before, "no patches strung along the jump");
        assert!(film_at(&slick, 1, 0.0, z + 300.0) > laid * 0.5);
    }

    /// A theatre's re-centre moves the slick whole; a re-lay carries each patch by kind.
    #[test]
    fn a_re_centre_and_a_re_lay_carry_the_slick() {
        struct Turned(f64, [f64; 2]);
        impl naval_render::relay::SceneRelay for Turned {
            fn point(&self, p: [f64; 3]) -> [f64; 3] {
                let [x, z] = self.direction(p, [p[0], p[2]]);
                [x + self.1[0], p[1], z + self.1[1]]
            }
            fn vector(&self, _: [f64; 3], v: [f64; 3]) -> [f64; 3] {
                let (sin, cos) = self.0.sin_cos();
                [v[0] * cos - v[2] * sin, v[1], v[2] * cos + v[0] * sin]
            }
            fn heading(&self, _: [f64; 3], heading: f64) -> f64 {
                heading + self.0
            }
        }
        let mut slick = OilSlick::new();
        let z = run(&mut slick, 30.0, 21.0, 0.3, [0.0; 2], 0.1, 0.0);
        let before = film_at(&slick, 1, 0.0, z + 300.0);
        let shift = [-30_616.0, 1_576.0];
        slick.translate(shift);
        let moved = film_at(&slick, 1, shift[0], z + 300.0 + shift[1]);
        assert!(
            (moved / before - 1.0).abs() < 0.05,
            "{moved} against {before}"
        );
        assert_eq!(film_at(&slick, 1, 0.0, z + 300.0), 0.0);

        let relay = Turned(0.3, [1_000.0, -2_000.0]);
        slick.relay(&relay);
        let to = relay.point([shift[0], 0.0, z + 300.0 + shift[1]]);
        let carried = film_at(&slick, 1, to[0], to[2]);
        assert!(
            (carried / before - 1.0).abs() < 0.1,
            "{carried} against {before}"
        );
        // Her last heading turned with the grid, so her next step is no turn of 0.3 rad in a tick.
        assert!((slick.previous.unwrap().0.heading - 0.3).abs() < 1e-12);
    }

    /// Every rasterisation's version is new, so another slick in the same slot repaints it.
    #[test]
    fn versions_are_unique_across_slicks() {
        let mut a = OilSlick::new();
        let mut b = OilSlick::new();
        run(&mut a, 2.0, 21.0, 0.3, [0.0; 2], 0.1, 0.0);
        run(&mut b, 2.0, 21.0, 0.3, [0.0; 2], 0.1, 0.0);
        assert_ne!(a.raster_version, b.raster_version);
        assert!(a.raster_version > 0 && b.raster_version > 0);
    }
}
