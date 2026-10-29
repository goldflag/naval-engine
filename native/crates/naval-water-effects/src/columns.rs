//! Shell splashes as columns of water marched through on the GPU (native; on 2026-10-16 they replaced the web game's
//! fan of camera-facing jets, its billows of gas puffs and its mist sprites, `native/docs/water-effects.md`, "Splash
//! columns"). A splash is one box about its water, drawn by `shaders/column.wgsl`, which follows each view ray
//! through the water's density (`shaders/column_field.wgsl`) and lights it as the clouds are lit; its shadow is
//! `shaders/column_caster.wgsl`.
//!
//! The water is where ballistics puts it. Every drop of the column leaves the cavity together, slowed by a drag in
//! proportion to its speed, so at time t the column is its launch speeds scaled by the flight's travel
//! T = (1 − e^(−kt)) / k and lowered by its fall (g / k)(t − T) ([`flight`]): it shoots up, hangs, and falls back from
//! the sea up, its slow water first. Around it hangs the veil of mist it sheds, which the wind carries off; at its foot
//! the crown thrown out sideways at the strike, and later the surge of mist its falling water raises off the sea.
//!
//! Every splash is the canonical 38 cm splash scaled, its lengths by its size and its times by the root of it, and
//! varied: no two shells strike the sea alike, so each throws its water at its own speed, girth and pace, lobes of its
//! column faster than the rest ([`Variety`]). This file works out each splash's flight once a frame in those terms
//! ([`ColumnGpu`]) and the box its water fills; the shaders hold the shapes. The constants shared with
//! `column_field.wgsl` are marked; keep the two in step.

use naval_effects::CameraView;
use naval_effects::math::smooth;
use std::f64::consts::TAU;

use bytemuck::{Pod, Zeroable};
use glam::{DVec2, DVec3};

pub const GRAVITY: f64 = 9.81;
/// The column's drag (1/s) and its head's launch speed (m/s): a 38 cm splash stands about 64 m at 2.6 s and its head
/// is back in the sea at about 8.5 s (`column_field.wgsl`'s `COLUMN_SPEED`).
pub const COLUMN_DRAG: f64 = 0.7;
pub const COLUMN_SPEED: f64 = 70.0;
/// The cavity the water leaves from, and the column's outward speed at its foot and how much faster its head opens
/// (`column_field.wgsl`: `COLUMN_ROOT`, `FOOT_SPREAD`, `HEAD_SPREAD`).
const COLUMN_ROOT: f64 = 2.5;
const FOOT_SPREAD: f64 = 2.2;
const HEAD_SPREAD: f64 = 8.0;
/// The skirt of the slowest water about the column's foot (`SKIRT_SPREAD`, `SKIRT_SHARE`).
const SKIRT_SPREAD: f64 = 6.5;
const SKIRT_SHARE: f64 = 0.25;
/// The column's fan (`column_field.wgsl`, 2026-10-23): the water leaves the cavity's rim as a hollow cone of sheet that
/// tears at once into `FEATHERS` feathers, each a bundle of streaks, with sky between them. The tallest, from the
/// cavity's middle, stands within `TALLEST_TILT` of the vertical; the others ring the cone, each its own way round
/// (evenly, give or take `RING_JITTER` of their spacing), leaning `RING_TILT` off the vertical (rad, × the root of the
/// splash's flare). Their half-widths (rad, × the root of its girth), and their tips' speeds over the splash's
/// (`TIP_TILT`: a tip leaning off the vertical is slower, as exp(−(lean / 0.62)²)).
pub const FEATHERS: usize = 8;
const TALLEST_TILT: f64 = 0.06;
const RING_TILT: [f64; 2] = [0.15, 0.5];
const RING_JITTER: f64 = 0.35;
const FEATHER_WIDTHS: [f64; 2] = [0.03, 0.075];
const TIPS: [f64; 2] = [0.72, 1.0];
const TIP_TILT: f64 = 0.85;
/// The cavity's rim the fan's water leaves from (m, × girth), and the farthest its water reaches from the axis, a share
/// of the splash's launch speed times the travel (the greatest exp(−(θ / 0.62)²) sin θ, about 0.26, its tips 1.15 of
/// its speed and spreading two half-widths) (`column_field.wgsl`'s `SPLASH_ROOT` and `SPLASH_REACH`).
const FAN_ROOT: f64 = 3.0;
const FAN_REACH: f64 = 0.55;
/// Its spray reaches this much past its fastest water's height (`column_field.wgsl`'s `along_share`, under 2 % beyond).
const TIP_END: f64 = 1.15;
/// The head opens as it falls (a share of its radius a second past `OPENS_AFTER`), and the column breaks up into spray
/// between these times (s).
const OPENING: f64 = 0.07;
const OPENS_AFTER: f64 = 1.6;
const BREAKUP: [f64; 2] = [1.2, 4.5];
/// The veil: the column's shape when it stops rising (s), how much wider it stands (`VEIL_WIDTH`), how it swells (a
/// share of its radius a second) and settles (m/s), and when it fades (it holds until `VEIL_HOLD`, then fades over
/// `VEIL_FADE` seconds). The wind carries it from `VEIL_DRIFTS` (s), when the mist has left the jets.
const VEIL_SHED: f64 = 2.4;
const VEIL_WIDTH: f64 = 1.55;
const VEIL_SWELL: f64 = 0.07;
const VEIL_SETTLE: f64 = 0.6;
const VEIL_HOLD: f64 = 3.5;
const VEIL_FADE: f64 = 3.5;
const VEIL_DRIFTS: [f64; 2] = [0.4, 2.0];
/// The crown's drag, its fastest launch (m/s) and its life (s).
const CROWN_DRAG: f64 = 1.3;
const CROWN_FAST: f64 = 30.0;
const CROWN_LIFE: [f64; 2] = [0.6, 1.6];
/// The surge: when the column's falling water first raises it (s), how far its rim rolls out (m past the cavity, over
/// `SURGE_TIME` s), its rim's width (m, widening a metre a second), its height (m) and how fast it fades (s).
const SURGE_START: f64 = 2.8;
const SURGE_REACH: f64 = 12.0;
const SURGE_TIME: f64 = 3.5;
const SURGE_WIDTH: f64 = 8.0;
const SURGE_WIDEN: f64 = 0.9;
const SURGE_HEIGHT: f64 = 10.0;
const SURGE_FADE: f64 = 4.5;
/// How long the canonical splash lasts (s): its veil and surge are gone by then (under 3 % of either left).
pub const LIFE: f64 = 16.0;
/// The box reaches this far below the strike's water level (canonical m), for the troughs of a sea.
const BELOW: f64 = 3.0;
/// How far splashes differ, the ranges each one's are drawn from: its speed (how high it stands), girth, head's flare
/// and pace (how soon it opens and tears), shares of the canonical splash's.
const SPEEDS: [f64; 2] = [0.74, 1.08];
const GIRTHS: [f64; 2] = [0.7, 1.4];
const FLARES: [f64; 2] = [0.5, 1.5];
const PACES: [f64; 2] = [0.8, 1.25];
/// How much faster than the splash's speed its tallest feather is thrown (×0.22: its tip 1–1.07 of it, the Houdini
/// trial's), most a little and a few in tall spires.
const SPIRES: [f64; 2] = [0.06, 0.32];
/// How far a column leans astray of the shell's way, whatever its fall (m per m risen), how much veil, crown and surge
/// it throws, and how far its surge rolls out (with its girth) and how high it stands.
const ASTRAY: f64 = 0.08;
const MISTS: [f64; 2] = [0.6, 1.35];
const ROLLS: [f64; 2] = [0.75, 1.3];
const ROLL_HEIGHTS: [f64; 2] = [0.6, 1.4];

/// The flight's travel T and fall at `t` seconds under a drag of `drag` per second: a launch velocity v is at
/// v·T − (0, fall, 0) from where it left.
pub fn flight(t: f64, drag: f64) -> (f64, f64) {
    let travel = -(-drag * t).exp_m1() / drag;
    (travel, GRAVITY / drag * (t - travel))
}

/// A splash's size (its lengths over the canonical 38 cm splash's) from its scale (1 for a 38 cm shell): small guns'
/// splashes stand taller than their calibre alone would make them.
pub fn size_of(scale: f64) -> f64 {
    scale.powf(0.65)
}

/// The column's radius at a share of the head's launch speed, at a travel T, as wide and its head as open as `variety`
/// throws them (`column_field.wgsl`'s `column_radius`).
fn column_radius(share: f64, travel: f64, variety: &Variety) -> f64 {
    let skirt = (1.0 - share / SKIRT_SHARE).max(0.0);
    variety.girth
        * (COLUMN_ROOT
            + travel
                * (FOOT_SPREAD
                    + HEAD_SPREAD * variety.flare * share * share * share
                    + SKIRT_SPREAD * skirt * skirt))
}

/// The `k`th draw (0–1) from a splash's own number. Hashed: the numbers of splashes struck one after another follow
/// one another (`naval_effects::random::Lcg::for_event`), their draws must not.
fn draw(seed: f64, k: u64) -> f64 {
    // splitmix64's finaliser.
    let mut x = seed.to_bits() ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn within(range: [f64; 2], d: f64) -> f64 {
    range[0] + (range[1] - range[0]) * d
}

/// How one splash differs from the canonical one, drawn from its own number: no two shells strike the sea alike (the
/// angle, the fuse, the sea it meets), and in the photographs no two columns of a salvo stand alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variety {
    /// Its water's launch speed over the canonical splash's: how high it stands.
    pub speed: f64,
    /// Its width, and how far its head opens, over the canonical splash's.
    pub girth: f64,
    pub flare: f64,
    /// How soon it opens and breaks up over the canonical splash (its clock runs this much faster for those).
    pub pace: f64,
    /// How much its tallest feather is thrown faster than the splash's speed (×0.22, a share of its speed).
    pub spires: f64,
    /// How much veil, crown and surge it throws, over the canonical splash's.
    pub mist: DVec3,
    /// How far its surge rolls out and how high it stands, over the canonical splash's.
    pub roll: DVec2,
}

impl Variety {
    /// The canonical splash itself.
    pub const CANONICAL: Variety = Variety {
        speed: 1.0,
        girth: 1.0,
        flare: 1.0,
        pace: 1.0,
        spires: 0.0,
        mist: DVec3::ONE,
        roll: DVec2::ONE,
    };

    /// The splash numbered `seed`'s.
    pub fn of(seed: f64) -> Self {
        let mut k = 0;
        let mut next = || {
            k += 1;
            draw(seed, k)
        };
        let speed = within(SPEEDS, next());
        let girth = within(GIRTHS, next());
        let flare = within(FLARES, next());
        let pace = within(PACES, next());
        let spires = within(SPIRES, next().powi(2));
        let mist = DVec3::new(
            within(MISTS, next()),
            within(MISTS, next()),
            within(MISTS, next()),
        );
        let roll = DVec2::new(girth * within(ROLLS, next()), within(ROLL_HEIGHTS, next()));
        Self {
            speed,
            girth,
            flare,
            pace,
            spires,
            mist,
            roll,
        }
    }

    /// Its fastest water's launch speed over the canonical splash's (its tallest feather's tip): none stands higher.
    pub fn reach(&self) -> f64 {
        self.speed * (1.0 + 0.22 * self.spires)
    }
}

/// One of a splash's feathers: its direction (unit, the splash's frame, +y up), its half-width (rad) and its tip's
/// launch speed (a share of `COLUMN_SPEED`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feather {
    pub axis: DVec3,
    pub width: f64,
    pub tip: f64,
}

/// Where round the vertical the ring of the splash numbered `seed` starts (rad): its `m`th feather (`m` from 0, the
/// feather after the tallest) stands `m` spacings of a turn over `FEATHERS − 1` on from it, give or take `RING_JITTER`
/// of a spacing (`column_field.wgsl` finds a point's two nearest by its bearing: `Column.kind.z`).
pub fn ring_start(seed: f64) -> f64 {
    TAU * draw(seed, 99)
}

/// The feathers of the splash numbered `seed` (`column_field.wgsl`'s `Column.feathers` and `Column.tips`), drawn from
/// its number past its variety's draws: the first its tallest, near the vertical, then the ring in order round it.
pub fn feathers(seed: f64, variety: &Variety) -> [Feather; FEATHERS] {
    std::array::from_fn(|i| {
        let k = 100 + 4 * i as u64;
        let lean = if i == 0 {
            TALLEST_TILT * draw(seed, k)
        } else {
            within(RING_TILT, draw(seed, k)) * variety.flare.sqrt()
        };
        let start = ring_start(seed);
        let turn = start
            + TAU * (i as f64 - 1.0 + RING_JITTER * (draw(seed, k + 1) - 0.5))
                / (FEATHERS - 1) as f64;
        let width = within(FEATHER_WIDTHS, draw(seed, k + 2)) * variety.girth.sqrt();
        let tip = if i == 0 {
            variety.reach()
        } else {
            variety.speed * within(TIPS, draw(seed, k + 3))
        } * (-(lean / TIP_TILT).powi(2)).exp();
        Feather {
            axis: DVec3::new(lean.sin() * turn.cos(), lean.cos(), lean.sin() * turn.sin()),
            width,
            tip,
        }
    })
}

/// What a splash of `size` shows `age` seconds after the strike, for the photo mode's shutter: its column's height
/// and radius (m) and how fast its water spreads out from its axis halfway up (m/s, slowing under the drag), or `None`
/// once the column is back in the sea.
pub fn column_extent(size: f64, age: f64) -> Option<(f64, f64, f64)> {
    let time = size.sqrt();
    let t = age / time;
    if t <= 0.0 {
        return None;
    }
    let (travel, fall) = flight(t, COLUMN_DRAG);
    let height = travel * COLUMN_SPEED - fall;
    if height < 1.0 {
        return None;
    }
    let radius = column_radius(0.6, travel, &Variety::CANONICAL);
    let spread = (FOOT_SPREAD + HEAD_SPREAD * 0.6f64.powi(3)) * (-COLUMN_DRAG * t).exp();
    // Speeds scale as lengths over times.
    Some((height * size, radius * size, spread * size / time))
}

/// One splash.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The point on the sea the shell struck (world m).
    pub origin: DVec3,
    /// Its lengths over the canonical splash's.
    pub size: f64,
    /// Seconds since the strike.
    pub age: f64,
    /// Its own number (0–1): where its lumps and streaks lie.
    pub seed: f64,
    /// How it differs from the canonical splash.
    pub variety: Variety,
    /// How far the column leans along the shell's way: horizontal metres per metre risen.
    pub lean: DVec2,
    /// How far the wind has carried its veil (world m).
    pub drift: DVec2,
    /// Its water's colour (linear; white unless dyed) and opacity.
    pub tint: DVec3,
    pub opacity: f64,
    /// Squared distance to the eye, for the back-to-front sort.
    distance: f64,
}

/// One splash as the shaders read it (`column_field.wgsl`'s `Column`; keep the two in order).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ColumnGpu {
    /// The strike's point on the sea (world m), and the splash's size.
    pub origin: [f32; 4],
    /// The box's lowest corner from the origin (world m), and the age in the canonical splash's seconds.
    pub low: [f32; 4],
    /// The box's highest corner, and the splash's own number.
    pub high: [f32; 4],
    /// The lean (xy), and the veil's drift (zw, canonical m).
    pub way: [f32; 4],
    /// The column's travel and fall (canonical m), how far its head has opened, how far it has broken up.
    pub flight: [f32; 4],
    /// The veil's travel and fall when shed, its swelling and its amount.
    pub veil: [f32; 4],
    /// How far the veil has settled, the crown's travel and fall, and its amount.
    pub crown: [f32; 4],
    /// The surge's reach, rim width, height and amount.
    pub surge: [f32; 4],
    /// The water's colour and opacity.
    pub tint: [f32; 4],
    /// Its variety's speed, girth and flare, and its fastest water's speed ([`Variety::reach`]).
    pub shape: [f32; 4],
    /// The box's frame on the sea: its x and z axes (world xz, unit). A splash's is the world's own, (1, 0, 0, 1); a
    /// bow's burst lies along her side ([`crate::bow_bursts`]).
    pub frame: [f32; 4],
    /// x what the record holds ([`KIND_SPLASH`], [`KIND_BURST`], [`KIND_SHEET`]: the shaders read its other lanes by
    /// it).
    pub kind: [f32; 4],
    /// A splash's feathers ([`feathers`]): each one's direction (xyz) and half-width (w); zero for a bow's spray.
    pub feathers: [[f32; 4]; FEATHERS],
    /// Their tips' launch speeds, four a lane (shares of [`COLUMN_SPEED`]).
    pub tips: [[f32; 4]; FEATHERS / 4],
}

/// A splash's record ([`ColumnGpu::kind`]), a bow's burst's and a bow's sheet's.
pub const KIND_SPLASH: f32 = 0.0;
pub const KIND_BURST: f32 = 1.0;
pub const KIND_SHEET: f32 = 2.0;

impl ColumnGpu {
    /// A point of its box's frame (m from its origin, along the frame's axes) in the world.
    pub fn world(&self, local: DVec3) -> DVec3 {
        let [ax, az, bx, bz] = self.frame.map(f64::from);
        DVec3::new(
            f64::from(self.origin[0]) + ax * local.x + bx * local.z,
            f64::from(self.origin[1]) + local.y,
            f64::from(self.origin[2]) + az * local.x + bz * local.z,
        )
    }

    /// Its box's middle (world) and the radius of the sphere about it.
    pub fn bounds(&self) -> (DVec3, f64) {
        let low = DVec3::new(
            f64::from(self.low[0]),
            f64::from(self.low[1]),
            f64::from(self.low[2]),
        );
        let high = DVec3::new(
            f64::from(self.high[0]),
            f64::from(self.high[1]),
            f64::from(self.high[2]),
        );
        (self.world((low + high) * 0.5), (high - low).length() * 0.5)
    }
}

impl Column {
    /// The column's age in the canonical splash's seconds.
    fn canonical_age(&self) -> f64 {
        self.age / self.size.sqrt()
    }

    /// Its flight at its age, and the box its water fills (from its origin, world m).
    pub fn state(&self) -> ColumnGpu {
        let t = self.canonical_age();
        let size = self.size;
        let drift = self.drift / size;
        let lean = self.lean;
        let variety = &self.variety;
        // The column.
        let (travel, fall) = flight(t, COLUMN_DRAG);
        let paced = t * variety.pace;
        let opening = OPENING * (paced - OPENS_AFTER).max(0.0);
        let breakup = smooth((paced - BREAKUP[0]) / (BREAKUP[1] - BREAKUP[0]));
        let mut low = DVec3::new(f64::MAX, -BELOW, f64::MAX);
        let mut high = DVec3::new(f64::MIN, 0.0, f64::MIN);
        let mut take = |center: DVec2, radius: f64, bottom: f64, top: f64| {
            low.x = low.x.min(center.x - radius);
            low.z = low.z.min(center.y - radius);
            high.x = high.x.max(center.x + radius);
            high.z = high.z.max(center.y + radius);
            low.y = low.y.min(bottom);
            high.y = high.y.max(top);
        };
        let fan = feathers(self.seed, variety);
        let lifted_top = travel * COLUMN_SPEED * variety.reach() * TIP_END;
        let top = lifted_top - fall;
        if top > 0.0 {
            let radius = variety.girth * FAN_ROOT
                + travel * COLUMN_SPEED * variety.reach() * FAN_REACH * (1.0 + opening);
            // Leaning, its foot and its head stand apart: the box takes both.
            take(lean * fall.min(lifted_top), radius, -BELOW, top);
            take(lean * lifted_top, radius, -BELOW, top);
        }
        // The veil.
        let shed = t.min(VEIL_SHED);
        let (veil_travel, veil_fall) = flight(shed, COLUMN_DRAG);
        let swelling = VEIL_SWELL * (t - 0.6).max(0.0);
        let veil = smooth((t - 0.25) / 1.35)
            * (-(t - VEIL_HOLD).max(0.0) / VEIL_FADE).exp()
            * variety.mist.x;
        let settled = VEIL_SETTLE * (t - VEIL_SHED).max(0.0);
        if veil > 0.002 {
            let lifted = 1.25 * veil_travel * COLUMN_SPEED * variety.speed;
            let radius = column_radius(1.0, veil_travel, variety).max(column_radius(
                0.0,
                veil_travel,
                variety,
            )) * VEIL_WIDTH
                * (1.0 + swelling);
            let bottom = -0.1 * veil_travel * COLUMN_SPEED - veil_fall - settled;
            take(
                drift + lean * lifted,
                radius,
                bottom,
                lifted - veil_fall - settled,
            );
            take(drift, radius, bottom, lifted - veil_fall - settled);
        }
        // The crown.
        let (crown_travel, crown_fall) = flight(t, CROWN_DRAG);
        let crown =
            (1.0 - smooth((t - CROWN_LIFE[0]) / (CROWN_LIFE[1] - CROWN_LIFE[0]))) * variety.mist.y;
        if crown > 0.0 {
            take(
                DVec2::ZERO,
                COLUMN_ROOT + crown_travel * CROWN_FAST,
                -BELOW,
                crown_travel * CROWN_FAST - crown_fall,
            );
        }
        // The surge.
        let since = t - SURGE_START;
        let (mut reach, mut width, mut height, mut surge) = (COLUMN_ROOT, SURGE_WIDTH, 1.0, 0.0);
        if since > 0.0 {
            reach = COLUMN_ROOT
                + (5.0 + SURGE_REACH * (1.0 - (-since / SURGE_TIME).exp())) * variety.roll.x;
            width = (SURGE_WIDTH + SURGE_WIDEN * since) * variety.roll.x;
            height = SURGE_HEIGHT * (0.5 + 0.5 * (1.0 - (-since / 2.5).exp())) * variety.roll.y;
            surge = smooth(since / 1.2) * (-since / SURGE_FADE).exp() * variety.mist.z;
            if surge > 0.002 {
                take(drift * 0.5, reach * 1.3 + 2.0 * width, -BELOW, 1.1 * height);
            }
        }
        if low.x > high.x {
            // Nothing left: an empty box at the strike.
            low = DVec3::ZERO;
            high = DVec3::ZERO;
        }
        let f = |v: f64| v as f32;
        ColumnGpu {
            origin: [
                f(self.origin.x),
                f(self.origin.y),
                f(self.origin.z),
                f(size),
            ],
            low: [f(low.x * size), f(low.y * size), f(low.z * size), f(t)],
            high: [
                f(high.x * size),
                f(high.y * size),
                f(high.z * size),
                f(self.seed),
            ],
            way: [f(lean.x), f(lean.y), f(drift.x), f(drift.y)],
            flight: [f(travel), f(fall), f(opening), f(breakup)],
            veil: [f(veil_travel), f(veil_fall), f(swelling), f(veil)],
            crown: [f(settled), f(crown_travel), f(crown_fall), f(crown)],
            surge: [f(reach), f(width), f(height), f(surge)],
            tint: [
                f(self.tint.x),
                f(self.tint.y),
                f(self.tint.z),
                f(self.opacity),
            ],
            shape: [
                f(variety.speed),
                f(variety.girth),
                f(variety.flare),
                f(variety.reach()),
            ],
            frame: [1.0, 0.0, 0.0, 1.0],
            kind: [KIND_SPLASH, 0.0, f(ring_start(self.seed)), 0.0],
            feathers: fan.map(|feather| {
                [
                    f(feather.axis.x),
                    f(feather.axis.y),
                    f(feather.axis.z),
                    f(feather.width),
                ]
            }),
            tips: std::array::from_fn(|lane| std::array::from_fn(|i| f(fan[lane * 4 + i].tip))),
        }
    }

    /// Whether any of its water is left.
    pub fn alive(&self) -> bool {
        self.canonical_age() < LIFE
    }
}

/// The splash columns standing: a bounded list, the oldest given up when it is full.
pub struct SplashColumns {
    columns: Vec<Column>,
    capacity: usize,
    published: Vec<ColumnGpu>,
}

impl SplashColumns {
    pub fn new(capacity: usize) -> Self {
        Self {
            columns: Vec::with_capacity(capacity),
            capacity,
            published: Vec::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The splashes standing.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Columns published last frame (in view).
    pub fn count(&self) -> usize {
        self.published.len()
    }

    /// The last published columns, back to front.
    pub fn instances(&self) -> &[ColumnGpu] {
        &self.published
    }

    /// A splash at `origin` (its water level), of `size`, the shell arriving along `direction` (unit), its own number
    /// `seed`.
    pub fn emit(&mut self, origin: DVec3, size: f64, direction: DVec3, seed: f64) {
        if self.columns.len() >= self.capacity {
            // Give up the oldest.
            if let Some(oldest) = self
                .columns
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.canonical_age().total_cmp(&b.1.canonical_age()))
                .map(|(i, _)| i)
            {
                self.columns.swap_remove(oldest);
            }
        }
        let steepness = direction.y.abs().min(1.0);
        let way = DVec2::new(direction.x, direction.z).normalize_or_zero();
        // A shell falling steeply throws its column straight up; a flatter one carries it along its way. Either leans a
        // little astray.
        let astray = TAU * draw(seed, 100);
        let lean = way * ((0.03 + 0.11 * (1.0 - steepness)) * within([0.8, 1.2], draw(seed, 101)))
            + DVec2::new(astray.cos(), astray.sin()) * (ASTRAY * draw(seed, 102));
        self.columns.push(Column {
            origin,
            size,
            age: 0.0,
            seed,
            variety: Variety::of(seed),
            lean,
            drift: DVec2::ZERO,
            tint: DVec3::ONE,
            opacity: 1.0,
            distance: 0.0,
        });
    }

    /// Age every splash by `dt`; the wind (m/s, world) carries the veils off.
    pub fn advance(&mut self, dt: f64, wind: DVec3) {
        for column in &mut self.columns {
            column.age += dt;
            let t = column.canonical_age();
            let carried = smooth((t - VEIL_DRIFTS[0]) / (VEIL_DRIFTS[1] - VEIL_DRIFTS[0]));
            column.drift += DVec2::new(wind.x, wind.z) * (dt * carried);
        }
        self.columns.retain(Column::alive);
    }

    /// The frame's columns in view, farthest first.
    pub fn publish(&mut self, camera: &CameraView) {
        self.published.clear();
        let (p0, p5) = (camera.projection.x_axis.x, camera.projection.y_axis.y);
        let mut shown: Vec<(f64, ColumnGpu)> = Vec::with_capacity(self.columns.len());
        for column in &mut self.columns {
            let state = column.state();
            let low = DVec3::new(
                f64::from(state.low[0]),
                f64::from(state.low[1]),
                f64::from(state.low[2]),
            );
            let high = DVec3::new(
                f64::from(state.high[0]),
                f64::from(state.high[1]),
                f64::from(state.high[2]),
            );
            if high.x <= low.x {
                continue;
            }
            let center = column.origin + (low + high) * 0.5;
            let radius = (high - low).length() * 0.5;
            let view = camera.view.transform_point3(camera.drawn(center));
            let depth = -view.z;
            if depth + radius < 0.0
                || view.x.abs() > depth.max(0.0) / p0 + radius * 1.5
                || view.y.abs() > depth.max(0.0) / p5 + radius * 1.5
            {
                continue;
            }
            column.distance = (center - camera.position).length_squared();
            shown.push((column.distance, state));
        }
        shown.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.published
            .extend(shown.into_iter().map(|(_, state)| state));
    }

    pub fn reset(&mut self) {
        self.columns.clear();
        self.published.clear();
    }
}

/// Spend full-resolution samples on small distant columns, bounded by their total projected box area. A near
/// column keeps the cheaper half-resolution march. The enclosing sphere is conservative, including a lens inside
/// a box; hysteresis keeps a slow camera movement from reallocating the target every frame.
pub(crate) fn march_divisor(
    columns: &[ColumnGpu],
    eye: glam::Vec3,
    forward: glam::Vec3,
    focal: f32,
    was_full: bool,
) -> u32 {
    let budget = if was_full { 80_000.0 } else { 64_000.0 };
    let mut pixels = 0.0;
    for column in columns {
        let (center, radius) = column.bounds();
        let (center, radius) = (center.as_vec3(), radius as f32);
        let depth = (center - eye).dot(forward) - radius;
        if depth <= 0.5 {
            return 2;
        }
        let diameter = 2.0 * radius * focal / depth;
        pixels += diameter * diameter;
        if pixels > budget {
            return 2;
        }
    }
    if columns.is_empty() { 2 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(t: f64) -> f64 {
        let (travel, fall) = flight(t, COLUMN_DRAG);
        travel * COLUMN_SPEED - fall
    }

    #[test]
    fn distant_columns_get_full_resolution_with_a_bounded_pixel_budget() {
        let column = ColumnGpu {
            low: [-30.0, 0.0, -30.0, 0.0],
            high: [30.0, 80.0, 30.0, 0.0],
            frame: [1.0, 0.0, 0.0, 1.0],
            ..ColumnGpu::zeroed()
        };
        let far = glam::Vec3::new(0.0, 40.0, 3000.0);
        let near = glam::Vec3::new(0.0, 40.0, 450.0);
        assert_eq!(
            march_divisor(&[column; 6], far, glam::Vec3::NEG_Z, 1900.0, false),
            1
        );
        assert_eq!(
            march_divisor(&[column; 6], near, glam::Vec3::NEG_Z, 1900.0, false),
            2
        );
        assert_eq!(
            march_divisor(&[column; 32], far, glam::Vec3::NEG_Z, 1900.0, false),
            2
        );
        assert_eq!(
            march_divisor(
                &[column],
                glam::Vec3::new(0.0, 40.0, 0.0),
                glam::Vec3::NEG_Z,
                1900.0,
                false
            ),
            2
        );
        assert_eq!(march_divisor(&[], far, glam::Vec3::NEG_Z, 1900.0, false), 2);
        // Between the enter and leave budgets either choice holds rather than oscillating.
        assert_eq!(
            march_divisor(&[column; 12], far, glam::Vec3::NEG_Z, 1900.0, false),
            2
        );
        assert_eq!(
            march_divisor(&[column; 12], far, glam::Vec3::NEG_Z, 1900.0, true),
            1
        );
    }

    #[test]
    fn the_column_shoots_up_hangs_and_falls_back() {
        // Most of its height within a second and a half, about 64 m at its top, back in the sea by nine seconds.
        assert!(top(0.5) > 25.0 && top(0.5) < 35.0, "{}", top(0.5));
        assert!(top(1.5) > 52.0, "{}", top(1.5));
        let peak = (1..200)
            .map(|i| top(f64::from(i) * 0.05))
            .fold(0.0, f64::max);
        assert!(peak > 60.0 && peak < 68.0, "{peak}");
        assert!(top(9.0) < 0.0, "{}", top(9.0));
        // Its foot's water falls back first: at five seconds the water launched under half the head's speed is gone.
        let (travel, fall) = flight(5.0, COLUMN_DRAG);
        assert!(fall / (travel * COLUMN_SPEED) > 0.5);
    }

    #[test]
    fn a_splash_is_the_canonical_one_scaled() {
        let mut big = Column {
            origin: DVec3::new(100.0, 0.0, -50.0),
            size: 1.0,
            age: 2.0,
            seed: 0.4,
            variety: Variety::CANONICAL,
            lean: DVec2::ZERO,
            drift: DVec2::ZERO,
            tint: DVec3::ONE,
            opacity: 1.0,
            distance: 0.0,
        };
        let mut small = big.clone();
        small.size = 0.25;
        // Half the root of its size: the same moment of its flight.
        small.age = 1.0;
        let (a, b) = (big.state(), small.state());
        assert!((a.low[3] - b.low[3]).abs() < 1e-6);
        for i in 0..3 {
            assert!((a.high[i] * 0.25 - b.high[i]).abs() < 1e-3, "{a:?} {b:?}");
        }
        // The box holds the column's head.
        assert!(f64::from(a.high[1]) > top(2.0));
        big.age = LIFE + 0.1;
        assert!(!big.alive());
    }

    #[test]
    fn the_box_follows_the_veil_downwind() {
        let mut columns = SplashColumns::new(8);
        columns.emit(DVec3::ZERO, 1.0, DVec3::NEG_Y, 0.5);
        for _ in 0..600 {
            columns.advance(1.0 / 60.0, DVec3::new(4.0, 0.0, 0.0));
        }
        let column = &columns.columns()[0];
        assert!(
            column.drift.x > 20.0 && column.drift.x < 40.0,
            "{}",
            column.drift.x
        );
        let state = column.state();
        assert!(
            state.high[0] > 40.0 && state.high[0] + state.low[0] > 0.0,
            "{state:?}"
        );
        // The surge has rolled out by now, the crown has fallen back.
        assert!(state.surge[3] > 0.0 && state.crown[3] == 0.0);
    }

    #[test]
    fn a_full_list_gives_up_its_oldest() {
        let mut columns = SplashColumns::new(2);
        columns.emit(DVec3::ZERO, 1.0, DVec3::NEG_Y, 0.1);
        columns.advance(1.0, DVec3::ZERO);
        columns.emit(DVec3::X, 1.0, DVec3::NEG_Y, 0.2);
        columns.advance(0.5, DVec3::ZERO);
        columns.emit(DVec3::Z, 1.0, DVec3::NEG_Y, 0.3);
        let seeds: Vec<f64> = columns.columns().iter().map(|c| c.seed).collect();
        assert_eq!(seeds.len(), 2);
        assert!(!seeds.contains(&0.1), "{seeds:?}");
    }

    #[test]
    fn no_two_splashes_stand_alike() {
        // Splashes struck one after another: their numbers follow one another, as `Lcg::for_event` gives them.
        let varieties: Vec<Variety> = (0..24)
            .map(|i| Variety::of((0.31 + 0.0694 * f64::from(i)).fract()))
            .collect();
        let speeds: Vec<f64> = varieties.iter().map(|v| v.speed).collect();
        let (low, high) = speeds
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), &s| (a.min(s), b.max(s)));
        assert!(high - low > 0.6 * (SPEEDS[1] - SPEEDS[0]), "{low} {high}");
        // Not in order: the draws do not follow the numbers.
        let rises = speeds.windows(2).filter(|w| w[1] > w[0]).count();
        assert!(rises > 5 && rises < 18, "{speeds:?}");
        // The box holds each one's fastest water.
        for (i, variety) in varieties.iter().enumerate() {
            let column = Column {
                origin: DVec3::ZERO,
                size: 1.0,
                age: 2.6,
                seed: 0.5,
                variety: *variety,
                lean: DVec2::ZERO,
                drift: DVec2::ZERO,
                tint: DVec3::ONE,
                opacity: 1.0,
                distance: 0.0,
            };
            let (travel, fall) = flight(2.6, COLUMN_DRAG);
            let fastest = travel * COLUMN_SPEED * variety.reach() - fall;
            assert!(f64::from(column.state().high[1]) >= fastest, "{i}");
        }
    }

    #[test]
    fn the_shutter_sees_the_column() {
        let (early, _, fast) = column_extent(1.0, 0.5).unwrap();
        let (later, radius, slow) = column_extent(1.0, 2.0).unwrap();
        assert!(
            early > 20.0 && later > early && later < 70.0,
            "{early} {later}"
        );
        assert!(radius > 4.0 && radius < 15.0, "{radius}");
        assert!(fast > slow);
        assert!(column_extent(1.0, 10.0).is_none());
        assert!(column_extent(1.0, 0.0).is_none());
        assert!(column_extent(size_of(0.127 / 0.38), 2.0).unwrap().0 < later * 0.8);
    }
}
