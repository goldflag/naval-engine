//! The `us-pacific-strip` template: a US airfield of 1943 in the South Pacific laid out on its site ([`us_pacific_strip`]),
//! in metres about the base's point. A crushed-coral runway (Marston mat over it on a fighter strip) with its shoulders,
//! overruns and the dust beyond its ends; a parallel taxiway joined to it unevenly at its ends and between; dispersal
//! loops beyond the taxiway (wobbling, some spurs with a hammerhead), rough coral hardstands along them, most in a U of
//! bulldozed earth facing the loop more or less, a warm-up apron by one end; aircraft parked on most hardstands and the
//! aprons ([`Parked`]); a perimeter road, a wooden control tower with its operations hut; and a camp in the plantation
//! off to one side: clusters of pyramidal tents and Quonset huts, mess halls, a motor pool, stores, each on its trampled
//! ground with a road to the camp's spine, trucks, jeeps, crates and drums about them; beyond it the fuel dumps and a
//! battery of 90 mm guns, 40 mm pits off the runway; tyre tracks across the open ground. Every piece is kept on dry,
//! fair ground ([`super::site::usable`]) and off the paving, every road joins another or the piece it serves, and every
//! choice is the base's seed's. What it clears of trees and buildings is [`Plan::clearing`]: the airfield whole; in
//! the camp and beyond it only what stands, its streets and its roads (soft: the land's own ground and trees left
//! between them, the plantation's palms the land's).

use std::f32::consts::{PI, TAU};

use glam::Vec2;

use super::site::{Site, SiteInput, usable};
use super::{Rng, SizeClass};

/// What a strip or patch of coral is (the shader's look, `Material::Coral`'s first colour byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Look {
    Runway = 0,
    Taxiway = 1,
    /// A hardstand or a lot.
    Hardstand = 2,
    Road = 3,
    /// Tyre tracks worn across the ground: two ruts in the earth.
    Track = 4,
    /// Trampled earth and coral dust (beyond a runway's ends, about a dump or a battery).
    Dust = 5,
    /// A trodden strip: a camp's street worn to earth.
    Trodden = 6,
    /// A hand-built runway (a Japanese field's): coral and gravel rolled over packed earth, patchier, its craters
    /// filled, its edges broken far into the ground beside it.
    Field = 7,
    /// Packed earth and gravel (a Japanese field's taxiways and roads): two wheel paths worn darker.
    Dirt = 8,
    /// Dark tarmac over concrete (an RAF station's runways, perimeter track and lanes): patched, worn paler where
    /// wheels run, its edge crisp into the grass.
    Tarmac = 9,
    /// Concrete in slabs (a Luftwaffe base's runways and taxiways): pale, its joints dark, tyre-marked.
    Concrete = 10,
    /// A hedgerow painted across a field's grass (the Luftwaffe's camouflage): dark green, broken.
    Hedge = 11,
    /// Grass mown shorter for a landing run, a shade paler, mown in stripes.
    Mown = 12,
    /// A round or rough pad of tarmac (an RAF frying pan), and of concrete.
    TarmacPad = 13,
    ConcretePad = 14,
}

/// What a strip is laid in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Coral,
    Marston,
}

/// A strip laid on the ground along a line: its whole half width and its paved half width (the rest its shoulders),
/// its look, its surface, its height over the ground (m: what lies higher covers what lies lower where they cross) and
/// the strips it is cut into across (a runway's four follow the ground across it).
#[derive(Clone, Debug, PartialEq)]
pub struct Ribbon {
    pub points: Vec<Vec2>,
    pub half: f32,
    pub paved: f32,
    pub look: Look,
    pub surface: Surface,
    pub lift: f32,
    pub across: u8,
}

impl Ribbon {
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }
}

/// A patch laid on the ground about `centre`: its outline (offsets from `centre`, every one seen from it), its look
/// (a hardstand or lot, or dust) and its height over the ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub centre: Vec2,
    pub ring: Vec<Vec2>,
    pub look: Look,
    pub lift: f32,
}

impl Pad {
    /// The farthest its outline reaches from its middle.
    pub fn reach(&self) -> f32 {
        self.ring.iter().map(|p| p.length()).fold(0.0, f32::max)
    }
}

/// A revetment: a U of bulldozed earth about a hardstand at `centre`, opening toward `facing` (unit); its inside's half
/// width, its berm's half width at the foot, its height (m), each arm's length (a share of the inside's half width:
/// the right arm's, then the left's, facing the opening), its colour and seed; `horseshoe`, its back a half round (a
/// Japanese field's, dug by hand) rather than square with its corners cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revetment {
    pub centre: Vec2,
    pub facing: Vec2,
    pub inner: f32,
    pub berm: f32,
    pub height: f32,
    pub arms: [f32; 2],
    pub colour: [u8; 3],
    pub seed: u32,
    pub horseshoe: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HutKind {
    /// A Quonset hut: a half-round of corrugated iron, plywood ends.
    Quonset,
    /// A frame building: plank walls, screened along their tops, a corrugated gable roof (a mess hall, an office).
    Frame,
    /// A ward tent (M1942): canvas walls and a canvas gable roof, 16 ft wide.
    Ward,
    /// A latrine: a plank box, a corrugated lean-to roof.
    Latrine,
    /// A Japanese barracks: long, low, of dark weathered planks, raised a little off the ground, shuttered windows
    /// down its sides, a pitched roof of boards or tar paper (`colour` the roof's, `ends` the walls').
    Barracks,
    /// A hut of the islands' kind: woven palm walls on a raised floor, a steep thatched roof down low over them.
    Thatch,
    /// A wooden hangar: plank walls, a gabled roof of iron, open at its end toward `axis`.
    Hangar,
    /// An RAF T2 hangar: corrugated steel walls and a low gabled roof on a steel frame, doors across both ends.
    T2,
    /// A blister hangar: an arch of corrugated steel to the ground, open at both ends.
    Blister,
    /// A brick building under a pitched roof (`colour` the roof's, `ends` the walls').
    Brick,
    /// A Luftwaffe barracks built to look like a farmhouse: rendered or brick walls, a steep roof of red tiles
    /// (`colour` the roof's, `ends` the walls').
    Farmhouse,
    /// An RAF watch office: two storeys of rendered brick, a flat roof railed round, a railed balcony across its
    /// front (toward `axis`), windows wide.
    WatchOffice,
    /// A Luftwaffe control building: two storeys of brick, a glazed room on its roof.
    Control,
    /// A Luftwaffe hangar: walls of brick or rendered concrete, a broad gabled roof, its doors across its long side
    /// toward `axis`.
    Hall,
}

/// A hut: its middle, its long axis (unit), its length, half width and height (m), its skin's colour and its ends'.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hut {
    pub centre: Vec2,
    pub axis: Vec2,
    pub length: f32,
    pub half: f32,
    pub height: f32,
    pub kind: HutKind,
    pub colour: [u8; 3],
    pub ends: [u8; 3],
}

/// A pyramidal tent (16 by 16 ft): its middle, its axis, half its side, its walls' and peak's heights (m), its canvas,
/// and whether its walls are rolled up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tent {
    pub centre: Vec2,
    pub axis: Vec2,
    pub half: f32,
    pub wall: f32,
    pub peak: f32,
    pub colour: [u8; 3],
    pub rolled: bool,
}

/// A wooden control tower: its foot's middle, its axis and the height of its cab's floor (m); `lookout`, an open
/// platform under a little roof atop a taller, slimmer frame and a ladder (a Japanese field's lookout).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tower {
    pub centre: Vec2,
    pub axis: Vec2,
    pub height: f32,
    pub lookout: bool,
}

/// A block of standing fuel drums: its middle, its axis, its drums along and across, and its tiers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drums {
    pub centre: Vec2,
    pub axis: Vec2,
    pub along: u8,
    pub across: u8,
    pub tiers: u8,
    pub seed: u8,
}

/// A drum's pitch in a block (m): 0.58 m across and a little room.
pub const DRUM_PITCH_M: f32 = 0.62;
/// A drum's height (m).
pub const DRUM_HEIGHT_M: f32 = 0.88;

/// A stack of wooden crates: its middle, its axis, its half length and width, its height (m) and the planks' colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crates {
    pub centre: Vec2,
    pub axis: Vec2,
    pub half: Vec2,
    pub height: f32,
    pub colour: [u8; 3],
}

/// A sandbag wall from `a` to `b`, its height (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wall {
    pub a: Vec2,
    pub b: Vec2,
    pub height: f32,
}

/// A duckboard walk: planks on runners from `a` to `b`, `half` its half width (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Board {
    pub a: Vec2,
    pub b: Vec2,
    pub half: f32,
}

/// A water tank: its middle, its radius, the height of the stand it sits on and its own (m); `tower` a tall stand of
/// braced timber, else a low one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tank {
    pub centre: Vec2,
    pub radius: f32,
    pub stand: f32,
    pub height: f32,
    pub tower: bool,
}

/// A straight bank of earth from `a` to `b` (a bomb dump's traverse, a blast pen's wall): its height, its half width
/// at the foot, its colour and seed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bank {
    pub a: Vec2,
    pub b: Vec2,
    pub height: f32,
    pub berm: f32,
    pub colour: [u8; 3],
    pub seed: u32,
}

/// A pillbox: a hexagon of concrete (`radius` to its corners, `height` its walls), loopholes in its faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pillbox {
    pub centre: Vec2,
    pub axis: Vec2,
    pub radius: f32,
    pub height: f32,
}

/// Camouflage over a revetment: netting on poles (a sheet sagging between them, `half` its half extents along `axis`
/// and across, `height` its poles'), or with `fronds` palm fronds laid over what stands there; its colour and seed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Net {
    pub centre: Vec2,
    pub axis: Vec2,
    pub half: Vec2,
    pub height: f32,
    pub colour: [u8; 3],
    pub fronds: bool,
    pub seed: u32,
}

/// A line of telephone poles beside a road: each pole's foot and height; a pair of wires sags from each to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct PoleLine {
    pub poles: Vec<(Vec2, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gun {
    /// A 90 mm M1 on its mount.
    Heavy,
    /// A 40 mm Bofors.
    Light,
    /// A Japanese navy 25 mm triple mount (Type 96), in a pit of earth.
    Triple,
    /// A Japanese heavier gun (a navy 12 cm high-angle gun, an army 75 mm), in a ring of earth.
    Navy,
    /// A Luftwaffe 2 cm Flak 38, in a ring of earth.
    Flak20,
    /// A Luftwaffe 8.8 cm Flak 36 on its cruciform mount, in a ring of earth.
    Flak88,
}

/// A gun pit: a ring of sandbags about `centre` (`radius` its middle line), its gap toward `opening`, its gun laid
/// toward `aim`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pit {
    pub centre: Vec2,
    pub radius: f32,
    pub opening: Vec2,
    pub gun: Gun,
    pub aim: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehicleKind {
    /// A 2½-ton truck, its bed under canvas.
    Truck,
    /// A ¾-ton weapons carrier.
    Carrier,
    /// A ¼-ton jeep.
    Jeep,
    /// A fuel bowser: a truck carrying a tank.
    Bowser,
    /// A hand cart: a flat bed on two wheels, its handles.
    Cart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vehicle {
    pub centre: Vec2,
    pub axis: Vec2,
    pub kind: VehicleKind,
    pub colour: [u8; 3],
}

/// An aircraft parked: where it stands (its middle), its heading (degrees clockwise from true north), its type (the
/// client's aircraft model's id), the ground under it (m above the sea, the coral's top), how that ground rises about it
/// (metres a metre along x and z) and its side number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parked {
    pub centre: Vec2,
    pub heading: f32,
    pub model: &'static str,
    pub ground: f32,
    pub slope: Vec2,
    pub number: u16,
}

/// The types a base parks (the client's aircraft models) and their wingspans (m, wings spread): the fighters, then the
/// bombers (1943 in the Solomons: the Marines' and the Navy's Corsairs, Wildcats and Hellcats, Dauntlesses and
/// Avengers).
pub const FIGHTERS: [(&str, f32); 3] = [
    ("f4u-1d-corsair", 12.5),
    ("f4f-4-wildcat", 11.6),
    ("f6f-5-hellcat", 13.1),
];
pub const BOMBERS: [(&str, f32); 2] = [("sbd-3-dauntless", 12.7), ("tbf-1c-avenger", 16.5)];

/// A piece of a base's clearing: no tree within `radius` of the segment from `a` to `b`, nor a building of the map's;
/// `soft`, its ground the land's own worn through in patches (what stands among the trees: a camp's huts and tents,
/// its streets and roads, a dump) rather than cleared to bare earth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub a: Vec2,
    pub b: Vec2,
    pub radius: f32,
    pub soft: bool,
}

impl Capsule {
    pub fn distance(&self, p: Vec2) -> f32 {
        segment_distance(p, self.a, self.b) - self.radius
    }
}

/// A base laid out, in metres about its point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub site: Option<Site>,
    pub ribbons: Vec<Ribbon>,
    pub pads: Vec<Pad>,
    pub revetments: Vec<Revetment>,
    pub huts: Vec<Hut>,
    pub tents: Vec<Tent>,
    pub towers: Vec<Tower>,
    pub drums: Vec<Drums>,
    pub crates: Vec<Crates>,
    pub walls: Vec<Wall>,
    pub boards: Vec<Board>,
    pub tanks: Vec<Tank>,
    pub poles: Vec<PoleLine>,
    pub nets: Vec<Net>,
    pub banks: Vec<Bank>,
    pub pillboxes: Vec<Pillbox>,
    pub pits: Vec<Pit>,
    pub vehicles: Vec<Vehicle>,
    pub parked: Vec<Parked>,
    /// Hardstands a plane may stand on (the dispersal's), of which [`Plan::parked`] fills most.
    pub stands: usize,
    pub clearing: Vec<Capsule>,
    /// A temperate base (Europe's): its cleared ground mown grass rather than bare earth.
    pub temperate: bool,
    /// What it all reaches (the clearing included): its corners.
    pub min: Vec2,
    pub max: Vec2,
}

impl Plan {
    /// Whether `p` lies in the base's clearing.
    pub fn holds(&self, p: Vec2) -> bool {
        p.cmpge(self.min).all()
            && p.cmple(self.max).all()
            && self.clearing.iter().any(|c| c.distance(p) <= 0.0)
    }

    /// The pieces standing (the strips, pads and aircraft not counted).
    pub fn pieces(&self) -> usize {
        self.revetments.len()
            + self.huts.len()
            + self.tents.len()
            + self.towers.len()
            + self.drums.len()
            + self.crates.len()
            + self.walls.len()
            + self.boards.len()
            + self.tanks.len()
            + self.poles.iter().map(|l| l.poles.len()).sum::<usize>()
            + self.nets.len()
            + self.banks.len()
            + self.pillboxes.len()
            + self.pits.len()
            + self.vehicles.len()
    }

    fn bound(&mut self) {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        let mut take = |p: Vec2, r: f32| {
            lo = lo.min(p - Vec2::splat(r));
            hi = hi.max(p + Vec2::splat(r));
        };
        for c in &self.clearing {
            take(c.a, c.radius);
            take(c.b, c.radius);
        }
        for r in &self.ribbons {
            for p in &r.points {
                take(*p, r.half);
            }
        }
        for p in &self.pads {
            take(p.centre, p.reach());
        }
        for r in &self.revetments {
            take(r.centre, r.inner * 2.0 + r.berm * 2.0);
        }
        for h in &self.huts {
            take(h.centre, h.length);
        }
        for t in &self.tents {
            take(t.centre, t.half * 2.0);
        }
        for t in &self.towers {
            take(t.centre, 8.0);
        }
        for d in &self.drums {
            take(d.centre, f32::from(d.along.max(d.across)) * DRUM_PITCH_M);
        }
        for c in &self.crates {
            take(c.centre, c.half.length());
        }
        for p in &self.pits {
            take(p.centre, p.radius + 6.0);
        }
        for v in &self.vehicles {
            take(v.centre, 5.0);
        }
        for p in &self.parked {
            take(p.centre, 10.0);
        }
        for w in &self.walls {
            take(w.a, 2.0);
            take(w.b, 2.0);
        }
        for b in &self.boards {
            take(b.a, 2.0);
            take(b.b, 2.0);
        }
        for t in &self.tanks {
            take(t.centre, t.radius + 3.0);
        }
        for line in &self.poles {
            for (p, _) in &line.poles {
                take(*p, 3.0);
            }
        }
        for n in &self.nets {
            take(n.centre, n.half.length() + 1.0);
        }
        for b in &self.banks {
            take(b.a, 2.0 * b.berm);
            take(b.b, 2.0 * b.berm);
        }
        for p in &self.pillboxes {
            take(p.centre, p.radius + 1.0);
        }
        if lo.x > hi.x {
            lo = Vec2::ZERO;
            hi = Vec2::ZERO;
        }
        self.min = lo;
        self.max = hi;
    }
}

/// The distance from `p` to the segment from `a` to `b`.
pub fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(nearest_on_segment(p, a, b))
}

fn nearest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    a + ab * t
}

/// The distance from `p` to a line through `points`.
pub fn line_distance(p: Vec2, points: &[Vec2]) -> f32 {
    match points {
        [] => f32::MAX,
        [a] => p.distance(*a),
        _ => points
            .windows(2)
            .map(|w| segment_distance(p, w[0], w[1]))
            .fold(f32::MAX, f32::min),
    }
}

/// The distance along `line` of each of its points.
pub(super) fn distances(line: &[Vec2]) -> Vec<f32> {
    let mut out = Vec::with_capacity(line.len());
    let mut run = 0.0;
    out.push(0.0);
    for w in line.windows(2) {
        run += w[0].distance(w[1]);
        out.push(run);
    }
    out
}

/// The point `at` metres along `line` (`along` its points' distances) and its direction there.
pub(super) fn point_at(line: &[Vec2], along: &[f32], at: f32) -> (Vec2, Vec2) {
    let i = along.partition_point(|a| *a <= at).clamp(1, line.len() - 1);
    let t = (at - along[i - 1]) / (along[i] - along[i - 1]).max(1e-3);
    (
        line[i - 1].lerp(line[i], t),
        (line[i] - line[i - 1]).normalize_or(Vec2::X),
    )
}

/// A line's corners cut `n` times (Chaikin), its ends kept.
pub fn smooth(points: &[Vec2], n: usize) -> Vec<Vec2> {
    let mut line = points.to_vec();
    for _ in 0..n {
        if line.len() < 3 {
            return line;
        }
        let mut next = Vec::with_capacity(line.len() * 2);
        next.push(line[0]);
        for w in line.windows(2) {
            next.push(w[0].lerp(w[1], 0.25));
            next.push(w[0].lerp(w[1], 0.75));
        }
        next.push(*line.last().expect("points"));
        // The first and last cut points stand in for the ends' own: drop them so the ends stay.
        next.remove(1);
        next.remove(next.len() - 2);
        line = next;
    }
    line
}

pub(super) fn rotate(v: Vec2, degrees: f32) -> Vec2 {
    Vec2::from_angle(degrees.to_radians()).rotate(v)
}

pub(super) fn perp(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x)
}

/// A direction's heading (degrees clockwise from true north; x east, y south).
pub(super) fn heading_of(d: Vec2) -> f32 {
    d.x.atan2(-d.y).to_degrees().rem_euclid(360.0)
}

/// A rough patch about the origin, `along` its half extent toward `facing` and `across` beside it: a rounded rectangle
/// (`squareness` 2 an ellipse, more squarer) its rim wandering by `wander` of itself.
pub(super) fn rough(
    rng: &mut Rng,
    facing: Vec2,
    along: f32,
    across: f32,
    squareness: f32,
    wander: f32,
    points: usize,
) -> Vec<Vec2> {
    let side = perp(facing);
    let start = rng.range(0.0, TAU);
    (0..points)
        .map(|k| {
            let t = start + k as f32 * TAU / points as f32;
            let (c, s) = (t.cos(), t.sin());
            let r = (c.abs().powf(squareness) / across.powf(squareness)
                + s.abs().powf(squareness) / along.powf(squareness))
            .powf(-1.0 / squareness);
            let r = r * (1.0 + wander * rng.signed());
            side * (c * r) + facing * (s * r)
        })
        .collect()
}

pub(super) const CANVAS: [[u8; 3]; 5] = [
    [112, 108, 74],
    [120, 114, 80],
    [104, 104, 72],
    [128, 120, 88],
    [98, 98, 68],
];
pub(super) const IRON: [[u8; 3]; 5] = [
    [150, 152, 148],
    [138, 140, 136],
    [160, 158, 150],
    [86, 88, 62],
    [98, 96, 70],
];
pub(super) const PLYWOOD: [[u8; 3]; 3] = [[140, 132, 112], [122, 120, 104], [112, 116, 92]];
pub(super) const PLANKS: [[u8; 3]; 3] = [[122, 108, 86], [112, 102, 82], [104, 98, 80]];
pub(super) const CRATE: [[u8; 3]; 3] = [[132, 116, 84], [118, 106, 78], [96, 94, 66]];
pub(super) const DRAB: [[u8; 3]; 3] = [[72, 74, 52], [80, 79, 57], [66, 70, 50]];
/// The berms' earth: Guadalcanal's red-brown, and greyer.
pub(super) const EARTH: [[u8; 3]; 3] = [[108, 90, 66], [118, 98, 72], [100, 88, 68]];

/// The template's heights over the ground (m): the mat over the coral, the coral over the taxiways, over the loops,
/// over the hardstands and lots, over the roads, over the tracks, over the dust.
pub(super) const MAT_LIFT: f32 = 0.30;
pub(super) const RUNWAY_LIFT: f32 = 0.22;
pub(super) const TAXIWAY_LIFT: f32 = 0.17;
pub(super) const LOOP_LIFT: f32 = 0.15;
pub(super) const PAD_LIFT: f32 = 0.12;
pub(super) const LOT_LIFT: f32 = 0.10;
pub(super) const ROAD_LIFT: f32 = 0.08;
pub(super) const TRACK_LIFT: f32 = 0.06;
pub(super) const DUST_LIFT: f32 = 0.04;
/// A runway's overrun of coral past each end (m).
const OVERRUN_M: f32 = 45.0;

/// The work of laying a base out: its site, what it may stand on, its generator, what it has laid so far and the
/// aircraft it parks (each type's id, wingspan and share).
pub(super) struct Layout<'a, 'b> {
    pub(super) site: Site,
    pub(super) input: &'a SiteInput<'b>,
    pub(super) rng: Rng,
    pub(super) plan: Plan,
    pub(super) fighter: bool,
    pub(super) air: Vec<(&'static str, f32, f32)>,
    /// The share of hardstands with a plane on them.
    pub(super) fill: f32,
    /// Whether what it clears now is soft (among the trees: the camp and beyond).
    pub(super) soft: bool,
    /// What its roads are laid as ([`Look::Road`], a Japanese field's [`Look::Dirt`]).
    pub(super) road_look: Look,
}

impl Layout<'_, '_> {
    pub(super) fn ground(&self, p: Vec2) -> f32 {
        (self.input.ground)(p)
    }

    pub(super) fn dry(&self, p: Vec2) -> bool {
        usable(self.input, p)
    }

    /// Dry about `p` within `r`, and no steeper there than `slope`.
    pub(super) fn fair(&self, p: Vec2, r: f32, slope: f32) -> bool {
        if !self.dry(p) {
            return false;
        }
        let h = self.ground(p);
        (0..6).all(|k| {
            let q = p + Vec2::from_angle(k as f32 * TAU / 6.0) * r;
            self.dry(q) && (self.ground(q) - h).abs() <= slope * r
        })
    }

    /// How far `p` lies outside every strip laid (its edge); tracks, dust and painted hedges not counted.
    pub(super) fn off_strips(&self, p: Vec2) -> f32 {
        let mut d = f32::MAX;
        for r in &self.plan.ribbons {
            if !matches!(r.look, Look::Track | Look::Hedge) {
                d = d.min(line_distance(p, &r.points) - r.half);
            }
        }
        d
    }

    /// How far `p` lies outside every strip laid but the tracks and a camp's trodden streets (which tents line).
    pub(super) fn off_roads(&self, p: Vec2) -> f32 {
        let mut d = f32::MAX;
        for r in &self.plan.ribbons {
            if !matches!(r.look, Look::Track | Look::Trodden | Look::Hedge) {
                d = d.min(line_distance(p, &r.points) - r.half);
            }
        }
        d
    }

    /// How far `p` lies from every hardstand, lot and standing piece laid (their edges, roughly).
    pub(super) fn off_pieces(&self, p: Vec2) -> f32 {
        let mut d = f32::MAX;
        for pad in &self.plan.pads {
            if pad.look != Look::Dust {
                d = d.min(p.distance(pad.centre) - pad.reach());
            }
        }
        for r in &self.plan.revetments {
            d = d.min(p.distance(r.centre) - (r.inner + 2.0 * r.berm));
        }
        for h in &self.plan.huts {
            let run = (0.5 * h.length - h.half).max(0.0);
            d = d.min(
                segment_distance(p, h.centre - h.axis * run, h.centre + h.axis * run) - h.half,
            );
        }
        for t in &self.plan.tents {
            d = d.min(p.distance(t.centre) - t.half * 1.42);
        }
        for t in &self.plan.towers {
            d = d.min(p.distance(t.centre) - 4.0);
        }
        for b in &self.plan.drums {
            d = d.min(p.distance(b.centre) - f32::from(b.along) * DRUM_PITCH_M * 0.6);
        }
        for c in &self.plan.crates {
            d = d.min(p.distance(c.centre) - c.half.length());
        }
        for pit in &self.plan.pits {
            d = d.min(p.distance(pit.centre) - pit.radius - 1.0);
        }
        for v in &self.plan.vehicles {
            d = d.min(p.distance(v.centre) - 3.5);
        }
        for a in &self.plan.parked {
            d = d.min(p.distance(a.centre) - 7.0);
        }
        for w in &self.plan.walls {
            d = d.min(segment_distance(p, w.a, w.b) - 1.0);
        }
        for b in &self.plan.boards {
            d = d.min(segment_distance(p, b.a, b.b) - b.half);
        }
        for t in &self.plan.tanks {
            d = d.min(p.distance(t.centre) - t.radius - 1.0);
        }
        for line in &self.plan.poles {
            for (q, _) in &line.poles {
                d = d.min(p.distance(*q) - 0.5);
            }
        }
        for b in &self.plan.banks {
            d = d.min(segment_distance(p, b.a, b.b) - 2.0 * b.berm);
        }
        for b in &self.plan.pillboxes {
            d = d.min(p.distance(b.centre) - b.radius);
        }
        d
    }

    pub(super) fn ribbon(&mut self, points: Vec<Vec2>, half: f32, look: Look, lift: f32) {
        if points.len() < 2 {
            return;
        }
        self.plan.ribbons.push(Ribbon {
            points,
            half,
            paved: half,
            look,
            surface: Surface::Coral,
            lift,
            across: 2,
        });
    }

    /// A road along `line` (bent a little), kept only where all of it is dry and clear of the pieces laid; its
    /// clearing a capsule a stretch.
    pub(super) fn road_along(&mut self, line: &[Vec2], half: f32, look: Look) -> bool {
        let mut sampled = Vec::new();
        for w in line.windows(2) {
            let n = (w[0].distance(w[1]) / 8.0).ceil().max(1.0) as usize;
            sampled.extend((0..n).map(|k| w[0].lerp(w[1], k as f32 / n as f32)));
        }
        sampled.extend(line.last());
        // A road keeps off what stands (a track is worn across anything).
        let clear = |p: &Vec2| look == Look::Track || self.off_pieces(*p) > half + 1.5;
        if sampled.len() < 2 || !sampled.iter().all(|p| self.dry(*p) && clear(p)) {
            return false;
        }
        let smoothed = smooth(line, 2);
        // Its clearing: one capsule, halved where the road bows more than 3 m off it (a winding road's capsule would
        // clear all it winds about).
        let mut spans = vec![(0, smoothed.len() - 1)];
        let mut k = 0;
        while k < spans.len() {
            let (i, j) = spans[k];
            let (a, b) = (smoothed[i], smoothed[j]);
            let bow = smoothed[i..=j]
                .iter()
                .map(|p| segment_distance(*p, a, b))
                .fold(0.0, f32::max);
            if bow > 3.0 && j - i >= 2 && spans.len() < 10 {
                let m = (i + j) / 2;
                spans[k] = (i, m);
                spans.push((m, j));
                continue;
            }
            // Among the trees its verges worn to earth, the trees kept back from them.
            let verge = if self.soft { 4.0 } else { 2.0 };
            self.plan.clearing.push(Capsule {
                a,
                b,
                radius: half + verge + bow,
                soft: self.soft,
            });
            k += 1;
        }
        let lift = match look {
            Look::Track => TRACK_LIFT,
            Look::Dirt | Look::Concrete | Look::Tarmac if half >= 4.0 => TAXIWAY_LIFT,
            _ => ROAD_LIFT,
        };
        self.ribbon(smoothed, half, look, lift);
        true
    }

    /// A road from `a` to `b`, bending a little in its middle.
    pub(super) fn road(&mut self, a: Vec2, b: Vec2, half: f32) -> bool {
        let mid = (a + b) * 0.5
            + perp((b - a).normalize_or_zero()) * self.rng.signed() * 0.06 * a.distance(b);
        self.road_along(&[a, mid, b], half, self.road_look)
    }

    /// The nearest point of a road or taxiway laid to `p`: where a road from `p` joins the others.
    pub(super) fn nearest_road(&self, p: Vec2) -> Option<Vec2> {
        let mut best: Option<(f32, Vec2)> = None;
        for r in &self.plan.ribbons {
            // Roads and taxiways, European paving but a runway (an RAF station's lanes and track, a Luftwaffe base's).
            let paving =
                matches!(r.look, Look::Tarmac | Look::Concrete) && r.lift < RUNWAY_LIFT - 0.01;
            if !matches!(r.look, Look::Road | Look::Taxiway | Look::Dirt) && !paving {
                continue;
            }
            for w in r.points.windows(2) {
                let q = nearest_on_segment(p, w[0], w[1]);
                let d = q.distance(p);
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, q));
                }
            }
        }
        best.map(|(_, q)| q)
    }

    /// A road from the edge of what stands within `reach` of `centre` to the nearest road laid, if all of it is dry.
    pub(super) fn join(&mut self, centre: Vec2, reach: f32, half: f32) -> bool {
        let Some(q) = self.nearest_road(centre) else {
            return false;
        };
        if q.distance(centre) < reach + 8.0 {
            return true;
        }
        // From as far in as it may come without running over what stands there.
        let toward = (q - centre).normalize_or_zero();
        for into in [0.3, 0.42, 0.55, 0.7, 0.85, 1.0] {
            let from = centre + toward * (into * reach);
            if self.road(from, q, half) {
                return true;
            }
        }
        false
    }

    /// A soft clearing about what stands from `a` to `b`, `radius` about it.
    pub(super) fn footprint(&mut self, a: Vec2, b: Vec2, radius: f32) {
        self.plan.clearing.push(Capsule {
            a,
            b,
            radius,
            soft: true,
        });
    }

    /// Whether `p` lies `margin` inside the airfield's own cleared ground (its hard capsules).
    pub(super) fn in_field(&self, p: Vec2, margin: f32) -> bool {
        self.plan
            .clearing
            .iter()
            .any(|c| !c.soft && c.distance(p) <= -margin)
    }

    /// Whether the clearing holds all within `r` of `p`.
    pub(super) fn covered(&self, p: Vec2, r: f32) -> bool {
        self.plan.clearing.iter().any(|c| c.distance(p) <= -r)
    }

    /// A type to park, by the base's shares.
    pub(super) fn pick_aircraft(&mut self) -> (&'static str, f32) {
        let total: f32 = self.air.iter().map(|a| a.2).sum();
        let mut x = self.rng.unit() * total;
        for &(id, span, share) in &self.air {
            if x < share {
                return (id, span);
            }
            x -= share;
        }
        let (id, span, _) = self.air[0];
        (id, span)
    }

    /// A plane parked at `centre` nose toward `facing` (a few degrees off), unless it would not fit `room` (half the
    /// width it may take).
    pub(super) fn park(&mut self, centre: Vec2, facing: Vec2, room: f32, lift: f32) {
        if self.air.is_empty() {
            return;
        }
        let (model, span) = self.pick_aircraft();
        if 0.5 * span > room {
            return;
        }
        let heading = heading_of(facing) + self.rng.signed() * 12.0;
        let number = 2 + self.rng.pick(38) as u16;
        let ground = self.ground(centre) + lift;
        let slope = Vec2::new(
            self.ground(centre + Vec2::X * 3.0) - self.ground(centre - Vec2::X * 3.0),
            self.ground(centre + Vec2::Y * 3.0) - self.ground(centre - Vec2::Y * 3.0),
        ) / 6.0;
        self.plan.parked.push(Parked {
            centre,
            heading,
            model,
            ground,
            slope,
            number,
        });
    }
}

/// The base laid out on `site`, as `input` (its seed, size, ground and neighbours) allows.
pub fn us_pacific_strip(site: Site, input: &SiteInput<'_>) -> Plan {
    let fighter = input.size == SizeClass::FighterStrip;
    let mut rng = Rng::new(input.seed, 1);
    // The squadrons it bases: a fighter strip one fighter type mostly and another, a bomber field Dauntlesses and
    // Avengers with a few fighters.
    let first = rng.pick(FIGHTERS.len());
    let second = (first + 1 + rng.pick(FIGHTERS.len() - 1)) % FIGHTERS.len();
    let air = if fighter {
        vec![
            (FIGHTERS[first].0, FIGHTERS[first].1, 0.8),
            (FIGHTERS[second].0, FIGHTERS[second].1, 0.2),
        ]
    } else {
        vec![
            (BOMBERS[0].0, BOMBERS[0].1, 0.5),
            (BOMBERS[1].0, BOMBERS[1].1, 0.3),
            (FIGHTERS[first].0, FIGHTERS[first].1, 0.2),
        ]
    };
    let fill = rng.range(0.6, 0.8);
    let mut layout = Layout::new(site, input, rng, fighter, air, fill);
    runway(&mut layout);
    let taxiway = taxiway(&mut layout);
    dispersal(&mut layout, taxiway);
    warm_up(&mut layout, taxiway);
    let road = perimeter(&mut layout);
    tower(&mut layout, road);
    // The camp and what lies beyond it stand among the trees: only their own ground cleared.
    layout.soft = true;
    if let Some(camp) = camp(&mut layout) {
        let mut spots = spots(&mut layout, &camp);
        dumps(&mut layout, &camp, &mut spots);
        battery(&mut layout, &camp, &mut spots);
    }
    light_pits(&mut layout);
    tracks(&mut layout, taxiway);
    layout.finish()
}

impl<'a, 'b> Layout<'a, 'b> {
    pub(super) fn new(
        site: Site,
        input: &'a SiteInput<'b>,
        rng: Rng,
        fighter: bool,
        air: Vec<(&'static str, f32, f32)>,
        fill: f32,
    ) -> Self {
        Layout {
            site,
            input,
            rng,
            plan: Plan {
                site: Some(site),
                temperate: input.template.temperate(),
                ..Plan::default()
            },
            fighter,
            air,
            fill,
            soft: false,
            road_look: Look::Road,
        }
    }

    /// The plan, every piece standing outside the clearing given its own footprint (soft), its bounds taken.
    pub(super) fn finish(mut self) -> Plan {
        footprints(&mut self);
        let mut plan = self.plan;
        plan.bound();
        plan
    }
}

/// A soft clearing about each piece the clearing does not yet hold (a hut along its length, the rest about their
/// middles), so no tree stands through it and the ground about it is worn; the trees stand between.
fn footprints(l: &mut Layout<'_, '_>) {
    let mut want: Vec<(Vec2, Vec2, f32)> = Vec::new();
    for h in &l.plan.huts {
        let run = (0.5 * h.length - h.half).max(0.0);
        want.push((
            h.centre - h.axis * run,
            h.centre + h.axis * run,
            h.half + 2.5,
        ));
    }
    for t in &l.plan.tents {
        want.push((t.centre, t.centre, t.half * 1.42 + 2.0));
    }
    for t in &l.plan.towers {
        want.push((t.centre, t.centre, 6.0));
    }
    for d in &l.plan.drums {
        let r = 0.5 * f32::from(d.along.max(d.across)) * DRUM_PITCH_M + 1.5;
        want.push((d.centre, d.centre, r));
    }
    for c in &l.plan.crates {
        want.push((c.centre, c.centre, c.half.length() + 1.5));
    }
    for v in &l.plan.vehicles {
        want.push((v.centre, v.centre, 4.5));
    }
    for t in &l.plan.tanks {
        want.push((t.centre, t.centre, t.radius + 2.0));
    }
    for w in &l.plan.walls {
        want.push((w.a, w.b, 1.8));
    }
    for b in &l.plan.boards {
        want.push((b.a, b.b, b.half + 1.0));
    }
    for p in &l.plan.pits {
        want.push((p.centre, p.centre, p.radius + 2.0));
    }
    for r in &l.plan.revetments {
        want.push((r.centre, r.centre, r.inner + 1.0));
    }
    for p in &l.plan.pads {
        want.push((p.centre, p.centre, p.reach() + 1.5));
    }
    for line in &l.plan.poles {
        for (p, _) in &line.poles {
            want.push((*p, *p, 1.0));
        }
    }
    for b in &l.plan.banks {
        want.push((b.a, b.b, 2.0 * b.berm + 1.0));
    }
    for p in &l.plan.pillboxes {
        want.push((p.centre, p.centre, p.radius + 2.0));
    }
    for (a, b, r) in want {
        if !(l.covered(a, r) && l.covered(b, r)) {
            l.footprint(a, b, r);
        }
    }
}

fn runway(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let [e0, e1] = s.ends();
    l.plan.ribbons.push(Ribbon {
        points: vec![e0 - s.dir * OVERRUN_M, e1 + s.dir * OVERRUN_M],
        half: s.half + s.shoulder,
        paved: s.half,
        look: Look::Runway,
        surface: Surface::Coral,
        lift: RUNWAY_LIFT,
        across: 4,
    });
    if l.fighter {
        l.plan.ribbons.push(Ribbon {
            points: vec![e0, e1],
            half: s.half,
            paved: s.half,
            look: Look::Runway,
            surface: Surface::Marston,
            lift: MAT_LIFT,
            across: 4,
        });
    }
    // The dust blown and tracked beyond each end.
    for (end, out) in [(e0, -s.dir), (e1, s.dir)] {
        let centre = end + out * (OVERRUN_M + 25.0);
        if l.fair(centre, 20.0, 0.12) {
            let half = s.half + s.shoulder;
            let ring = rough(&mut l.rng, out, 45.0, half + 18.0, 2.4, 0.18, 18);
            l.plan.pads.push(Pad {
                centre,
                ring,
                look: Look::Dust,
                lift: DUST_LIFT,
            });
        }
    }
    l.plan.clearing.push(Capsule {
        a: e0 - s.dir * (OVERRUN_M + 50.0),
        b: e1 + s.dir * (OVERRUN_M + 50.0),
        radius: s.half + s.shoulder + 55.0,
        soft: false,
    });
}

/// The parallel taxiway: its offset across (signed, toward the dispersal) and half width.
#[derive(Clone, Copy)]
struct Taxiway {
    offset: f32,
    half: f32,
}

fn taxiway(l: &mut Layout<'_, '_>) -> Taxiway {
    let s = l.site;
    let (len, side) = (s.length, s.side);
    let half = if l.fighter { 9.0 } else { 12.0 };
    let gap = if l.fighter { 62.0 } else { 95.0 };
    let offset = side * (s.half + s.shoulder + gap);
    let edge = side * (s.half - 4.0);
    // Each end joins the runway its own way: sharper or longer, nearer the end or farther in.
    let mut ends = [[Vec2::ZERO; 3]; 2];
    for (k, sign) in [-1.0f32, 1.0].into_iter().enumerate() {
        let (a1, a2, a3) = (
            l.rng.range(8.0, 40.0),
            l.rng.range(40.0, 85.0),
            l.rng.range(95.0, 165.0),
        );
        let bend = l.rng.range(0.35, 0.7);
        ends[k] = [
            s.at(sign * (0.5 * len - a1), edge),
            s.at(sign * (0.5 * len - a2), offset * bend),
            s.at(
                sign * (0.5 * len - a3),
                offset + side * l.rng.signed() * 4.0,
            ),
        ];
    }
    let line = smooth(
        &[
            ends[0][0], ends[0][1], ends[0][2], ends[1][2], ends[1][1], ends[1][0],
        ],
        3,
    );
    l.ribbon(line, half, Look::Taxiway, TAXIWAY_LIFT);
    for end in &ends {
        l.plan.clearing.push(Capsule {
            a: end[0],
            b: end[2],
            radius: half + 24.0,
            soft: false,
        });
    }
    // Exits between: square, or angled either way.
    let exits = if l.fighter {
        1 + l.rng.pick(2)
    } else {
        2 + l.rng.pick(2)
    };
    for k in 0..exits {
        let along = -0.5 * len + len * (k + 1) as f32 / (exits + 1) as f32 + l.rng.signed() * 60.0;
        let slant = match l.rng.pick(3) {
            0 => 0.0,
            1 => l.rng.range(25.0, 60.0),
            _ => -l.rng.range(25.0, 60.0),
        };
        let line = smooth(
            &[
                s.at(along, offset),
                s.at(along - 0.45 * slant, offset * 0.5),
                s.at(along - slant, edge),
            ],
            2,
        );
        let width = half * l.rng.range(0.8, 1.0);
        l.ribbon(line, width, Look::Taxiway, TAXIWAY_LIFT - 0.005);
        l.plan.clearing.push(Capsule {
            a: s.at(along, offset),
            b: s.at(along - slant, edge),
            radius: half + 20.0,
            soft: false,
        });
    }
    l.plan.clearing.push(Capsule {
        a: s.at(-0.5 * len + 80.0, offset),
        b: s.at(0.5 * len - 80.0, offset),
        radius: half + 24.0,
        soft: false,
    });
    // The ground between the runway and the taxiway.
    l.plan.clearing.push(Capsule {
        a: s.at(-0.5 * len + 40.0, 0.55 * offset),
        b: s.at(0.5 * len - 40.0, 0.55 * offset),
        radius: 0.45 * offset.abs() + 12.0,
        soft: false,
    });
    Taxiway { offset, half }
}

/// The dispersal beyond the taxiway: loops and spurs of coral, wobbling, with their hardstands and revetments.
fn dispersal(l: &mut Layout<'_, '_>, taxiway: Taxiway) {
    let s = l.site;
    let side = s.side;
    let loops = if l.fighter { 4 } else { 5 } + l.rng.pick(2);
    let (from, to) = (-0.5 * s.length + 100.0, 0.5 * s.length - 100.0);
    let slot = (to - from) / loops as f32;
    let base = taxiway.offset + side * (taxiway.half - 1.5);
    let half = if l.fighter { 6.5 } else { 8.5 };
    for k in 0..loops {
        let mid = from + slot * (k as f32 + 0.5) + l.rng.signed() * slot * 0.12;
        let width = slot * l.rng.range(0.5, 0.82);
        let depth = if l.fighter {
            l.rng.range(110.0, 200.0)
        } else {
            l.rng.range(150.0, 270.0)
        };
        let skew = l.rng.signed() * 0.4;
        let wobble = [
            l.rng.range(0.04, 0.16),
            l.rng.range(0.02, 0.1),
            l.rng.range(0.02, 0.07),
        ];
        let phase = [
            l.rng.range(0.0, TAU),
            l.rng.range(0.0, TAU),
            l.rng.range(0.0, TAU),
        ];
        let jitter: Vec<Vec2> = (0..=18)
            .map(|_| Vec2::new(l.rng.signed(), l.rng.signed()) * 3.0)
            .collect();
        let spur = l.rng.chance(0.25);
        let bend = l.rng.signed() * 0.3 * width;
        for scale in [1.0, 0.72, 0.5] {
            let depth = depth * scale;
            let path: Vec<Vec2> = if spur {
                smooth(
                    &[
                        s.at(mid, base),
                        s.at(mid + bend * 0.3, base + side * depth * 0.5) + jitter[3],
                        s.at(mid + bend, base + side * depth),
                    ],
                    2,
                )
            } else {
                let raw: Vec<Vec2> = (0..=18)
                    .map(|j| {
                        let t = PI * j as f32 / 18.0;
                        let swell = 1.0
                            + wobble[0] * (2.0 * t + phase[0]).sin()
                            + wobble[1] * (3.0 * t + phase[1]).sin();
                        let u = -t.cos() * 0.5 * width
                            + skew * t.sin() * 0.5 * width
                            + wobble[2] * width * (4.0 * t + phase[2]).sin() * t.sin();
                        let v = t.sin() * depth * swell;
                        let shake = if j == 0 || j == 18 {
                            Vec2::ZERO
                        } else {
                            jitter[j]
                        };
                        s.at(mid + u, base + side * v) + shake
                    })
                    .collect();
                smooth(&raw, 1)
            };
            // Dry all along, and clear of the strips already laid (but where it leaves the taxiway and, a loop, joins
            // it again).
            let (first, last) = (path[0], *path.last().expect("a path"));
            let ok = path.iter().all(|p| {
                let joining = p.distance(first) < 30.0 || (!spur && p.distance(last) < 30.0);
                l.dry(*p) && (joining || l.off_strips(*p) > half + 6.0)
            });
            if !ok {
                continue;
            }
            let inside = s.at(mid + skew * 0.25 * width, base + side * depth * 0.45);
            l.ribbon(path.clone(), half, Look::Taxiway, LOOP_LIFT);
            let mut ends = Vec::new();
            if spur {
                // The hammerhead, a little askew.
                let tip = *path.last().expect("a path");
                let along = (tip - path[path.len() - 2]).normalize_or_zero();
                let cross = rotate(perp(along), l.rng.signed() * 12.0) * (0.28 * width).max(40.0);
                let bar = vec![tip - cross * l.rng.range(0.7, 1.0), tip + cross];
                if bar.iter().all(|p| l.dry(*p)) {
                    l.ribbon(bar.clone(), half, Look::Taxiway, LOOP_LIFT - 0.005);
                    ends.push(bar);
                }
            }
            hardstands(l, &path, half, inside, spur);
            for bar in &ends {
                hardstands(l, bar, half, inside, true);
            }
            // The loop's clearing: its line in thirds, wide enough for its hardstands and their berms.
            let reach = half + if l.fighter { 40.0 } else { 58.0 };
            let marks = [0, path.len() / 3, 2 * path.len() / 3, path.len() - 1];
            for w in marks.windows(2) {
                let (a, b) = (path[w[0]], path[w[1]]);
                let bow = path[w[0]..=w[1]]
                    .iter()
                    .map(|p| segment_distance(*p, a, b))
                    .fold(0.0, f32::max);
                l.plan.clearing.push(Capsule {
                    a,
                    b,
                    radius: reach + bow,
                    soft: false,
                });
            }
            for bar in ends {
                l.plan.clearing.push(Capsule {
                    a: bar[0],
                    b: bar[1],
                    radius: reach,
                    soft: false,
                });
            }
            break;
        }
    }
}

/// Hardstands along `path` (its loop's middle `inside`; on a spur, both sides by turns): rough pads pushed out by a
/// bulldozer, most in a U of earth facing the path more or less, of the sizes the base's aircraft want, most with a
/// plane on them; kept on fair ground clear of the strips and of each other.
fn hardstands(l: &mut Layout<'_, '_>, path: &[Vec2], half: f32, inside: Vec2, both: bool) {
    let mut along = Vec::with_capacity(path.len());
    let mut total = 0.0;
    along.push(0.0);
    for w in path.windows(2) {
        total += w[0].distance(w[1]);
        along.push(total);
    }
    let soil = EARTH[(l.input.seed % EARTH.len() as u32) as usize];
    let mut at = 35.0 + l.rng.range(0.0, 14.0);
    let mut turn = 0;
    while at < total - 25.0 {
        // Its size: a fighter's, a dive bomber's, an Avenger's.
        let (width, depth) = if l.fighter {
            (l.rng.range(8.0, 10.5), l.rng.range(7.5, 10.0))
        } else if l.rng.chance(0.35) {
            (l.rng.range(11.0, 13.0), l.rng.range(9.5, 12.0))
        } else {
            (l.rng.range(9.0, 11.0), l.rng.range(8.0, 10.5))
        };
        let berm = l.rng.range(0.85, 1.15) * if l.fighter { 4.4 } else { 5.3 };
        let i = along.partition_point(|a| *a <= at).clamp(1, path.len() - 1);
        let t = (at - along[i - 1]) / (along[i] - along[i - 1]).max(1e-3);
        let q = path[i - 1].lerp(path[i], t);
        let tangent = (path[i] - path[i - 1]).normalize_or_zero();
        let mut out = perp(tangent);
        if both {
            if turn % 2 == 1 {
                out = -out;
            }
        } else if out.dot(q - inside) < 0.0 {
            out = -out;
        }
        turn += 1;
        // Not quite square to the path: pushed out at an angle of its own.
        let facing = rotate(-out, l.rng.signed() * 28.0);
        let reach = width.max(depth);
        let centre = q + out * (half + depth - 1.5);
        let wall = reach + 1.5 + 2.0 * berm;
        let fits = l.fair(centre, wall, 0.12)
            && l.plan
                .ribbons
                .iter()
                .all(|r| r.points == path || line_distance(centre, &r.points) > r.half + wall)
            && l.off_pieces(centre) > wall + 2.0;
        if fits && !l.rng.chance(0.15) {
            let ring = rough(&mut l.rng, facing, depth, width, 3.5, 0.08, 12);
            l.plan.pads.push(Pad {
                centre,
                ring,
                look: Look::Hardstand,
                lift: PAD_LIFT,
            });
            l.plan.stands += 1;
            if l.rng.chance(0.85) {
                let colour = if l.rng.chance(0.25) {
                    EARTH[l.rng.pick(EARTH.len())]
                } else {
                    soil
                };
                l.plan.revetments.push(Revetment {
                    centre,
                    facing,
                    inner: reach + l.rng.range(1.0, 3.0),
                    berm,
                    height: if l.fighter { 2.3 } else { 2.7 } + l.rng.signed() * 0.4,
                    arms: [l.rng.range(0.25, 0.85), l.rng.range(0.25, 0.85)],
                    colour,
                    seed: l.rng.next_u64() as u32,
                    horseshoe: false,
                });
            }
            if l.rng.chance(l.fill) {
                // Nose out, toward the loop, its tail in the U.
                let back = centre - facing * (0.15 * depth);
                l.park(back, facing, width + 1.0, PAD_LIFT);
            }
            at += 2.0 * (reach + 2.0 * berm) + l.rng.range(4.0, 30.0);
        } else {
            at += 12.0;
        }
    }
}

/// A warm-up apron between the runway and the taxiway near one end: a lot with planes in echelon facing the runway.
fn warm_up(l: &mut Layout<'_, '_>, taxiway: Taxiway) {
    let s = l.site;
    let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    let inner = s.half + s.shoulder + 10.0;
    let outer = taxiway.offset.abs() - taxiway.half - 8.0;
    if outer - inner < 26.0 {
        return;
    }
    let along = sign * (0.5 * s.length - l.rng.range(170.0, 260.0));
    let across = s.side * 0.5 * (inner + outer);
    let centre = s.at(along, across);
    let half = Vec2::new(l.rng.range(45.0, 70.0), 0.5 * (outer - inner));
    if !l.fair(centre, half.y, 0.08) {
        return;
    }
    let ring = rough(&mut l.rng, s.dir, half.x, half.y, 6.0, 0.03, 20);
    l.plan.pads.push(Pad {
        centre,
        ring,
        look: Look::Hardstand,
        lift: LOT_LIFT,
    });
    // Toward the runway, turned for the end they will take off from.
    let toward = rotate(-s.side * s.right(), sign * s.side * 35.0);
    let count = 2 + l.rng.pick(3);
    for k in 0..count {
        let u = -half.x + 14.0 + (k as f32 + 0.5) * (2.0 * half.x - 28.0) / count as f32;
        if l.rng.chance(0.85) {
            let at = centre + s.dir * u + s.right() * (l.rng.signed() * 3.0);
            l.park(at, toward, half.y.min(10.0), LOT_LIFT);
        }
    }
}

/// The perimeter road on the side away from the dispersal: its offset across.
fn perimeter(l: &mut Layout<'_, '_>) -> f32 {
    let s = l.site;
    let offset = -s.side * (s.half + s.shoulder + 20.0);
    // Its ends turn in to the runway's coral past its ends.
    let edge = -s.side * (s.half + s.shoulder - 3.0);
    let line = smooth(
        &[
            s.at(-0.5 * s.length - 15.0, edge),
            s.at(-0.5 * s.length + 40.0, offset),
            s.at(0.5 * s.length - 40.0, offset),
            s.at(0.5 * s.length + 15.0, edge),
        ],
        2,
    );
    l.ribbon(line, 3.0, Look::Road, ROAD_LIFT);
    offset
}

/// The control tower by the perimeter road, its operations hut beside it.
fn tower(l: &mut Layout<'_, '_>, road: f32) {
    let s = l.site;
    let along = l.rng.signed() * 0.12 * s.length;
    let across = road - s.side * 14.0;
    let centre = s.at(along, across);
    if !l.fair(centre, 6.0, 0.15) {
        return;
    }
    l.plan.towers.push(Tower {
        centre,
        axis: s.dir,
        height: 9.5,
        lookout: false,
    });
    let hut = s.at(along + 16.0, across - s.side * 4.0);
    if l.fair(hut, 8.0, 0.15) {
        l.plan.huts.push(Hut {
            centre: hut,
            axis: s.dir,
            length: 14.6,
            half: 3.05,
            height: 3.3,
            kind: HutKind::Quonset,
            colour: IRON[l.rng.pick(3)],
            ends: PLYWOOD[l.rng.pick(PLYWOOD.len())],
        });
    }
    l.plan.clearing.push(Capsule {
        a: centre,
        b: hut,
        radius: 22.0,
        soft: false,
    });
    // A jeep and a truck or two by the operations hut, not in a row.
    for k in 0..l.rng.pick(3) + 1 {
        let p = s.at(
            along + 30.0 + k as f32 * 5.0,
            across - s.side * l.rng.range(0.0, 4.0),
        );
        if l.fair(p, 4.0, 0.15) {
            l.plan.vehicles.push(Vehicle {
                centre: p,
                axis: rotate(s.right() * -s.side, l.rng.signed() * 30.0),
                kind: [VehicleKind::Truck, VehicleKind::Jeep, VehicleKind::Carrier][k % 3],
                colour: DRAB[l.rng.pick(DRAB.len())],
            });
        }
    }
}

/// The camp: its middle, its axes (along the runway, and away from it) and its size.
struct Camp {
    centre: Vec2,
    u: Vec2,
    v: Vec2,
    size: Vec2,
}

impl Camp {
    /// The point `u` metres along the camp from its middle and `v` away from the runway.
    fn at(&self, u: f32, v: f32) -> Vec2 {
        self.centre + self.u * u + self.v * v
    }
}

/// The places about the camp for its fuel dumps and its heavy battery, in the seed's order: beyond its far side, past
/// its ends.
fn spots(l: &mut Layout<'_, '_>, camp: &Camp) -> Vec<Vec2> {
    let (w, d) = (camp.size.x, camp.size.y);
    let mut out = vec![
        camp.at(0.25 * w, 0.5 * d + 55.0),
        camp.at(-0.25 * w, 0.5 * d + 55.0),
        camp.at(0.5 * w + 70.0, 0.1 * d),
        camp.at(-0.5 * w - 70.0, 0.1 * d),
        camp.at(0.5 * w + 70.0, -0.3 * d),
        camp.at(-0.5 * w - 70.0, -0.3 * d),
        camp.at(0.0, 0.5 * d + 65.0),
    ];
    for k in (1..out.len()).rev() {
        let j = l.rng.pick(k + 1);
        out.swap(k, j);
    }
    out
}

/// The first of `spots` with room for `radius` of fair ground clear of what is laid, taken from them.
fn take_spot(l: &Layout<'_, '_>, spots: &mut Vec<Vec2>, radius: f32) -> Option<Vec2> {
    let k = spots.iter().position(|p| {
        l.fair(*p, radius, 0.1)
            && l.off_strips(*p) > radius + 10.0
            && l.off_pieces(*p) > radius + 5.0
    })?;
    Some(spots.remove(k))
}

fn camp(l: &mut Layout<'_, '_>) -> Option<Camp> {
    let s = l.site;
    let side = s.side;
    let full = if l.fighter {
        Vec2::new(300.0, 200.0)
    } else {
        Vec2::new(380.0, 250.0)
    };
    let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    let deepest = l
        .plan
        .clearing
        .iter()
        .map(|c| {
            ((c.a - s.centre).dot(s.right()) * side).max((c.b - s.centre).dot(s.right()) * side)
                + c.radius
        })
        .fold(0.0, f32::max);
    for scale in [1.0, 0.75] {
        let size = full * scale;
        let away = -side * s.right();
        let near = s.half + s.shoulder + 50.0 + 0.5 * size.y;
        // Beside the runway away from the dispersal (toward either end), beyond either end, past the dispersal.
        let candidates = [
            (s.at(sign * 0.2 * s.length, -side * near), away),
            (s.at(-sign * 0.2 * s.length, -side * near), away),
            (s.at(0.0, -side * (near + 0.4 * size.y)), away),
            (
                s.at(
                    sign * (0.5 * s.length + 70.0 + 0.5 * size.x),
                    -side * 0.3 * size.y,
                ),
                away,
            ),
            (
                s.at(
                    -sign * (0.5 * s.length + 70.0 + 0.5 * size.x),
                    -side * 0.3 * size.y,
                ),
                away,
            ),
            (
                s.at(
                    sign * 0.15 * s.length,
                    side * (deepest + 30.0 + 0.5 * size.y),
                ),
                -away,
            ),
        ];
        for (centre, away) in candidates {
            let turn = l.rng.signed() * 6.0;
            let u = rotate(s.dir, turn);
            let mut v = perp(u);
            if v.dot(away) < 0.0 {
                v = -v;
            }
            let camp = Camp { centre, u, v, size };
            // Mostly dry and fair over a grid of its ground, clear of the strips and the dispersal.
            let mut heights = Vec::new();
            let mut wet = 0;
            let mut ok = true;
            for i in 0..=6 {
                for j in 0..=4 {
                    let p = camp.at(
                        (i as f32 / 6.0 - 0.5) * size.x,
                        (j as f32 / 4.0 - 0.5) * size.y,
                    );
                    if l.dry(p) {
                        heights.push(l.ground(p));
                    } else {
                        wet += 1;
                    }
                    ok &= l.off_strips(p) > 12.0 && l.off_pieces(p) > 15.0;
                }
            }
            let (lo, hi) = heights
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
            if ok && wet <= 3 && hi - lo < 0.06 * size.x {
                lay_camp(l, &camp);
                return Some(camp);
            }
        }
    }
    None
}

/// What a cluster of the camp holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cluster {
    Tents,
    Quonsets,
    Mess,
    Motor,
    Stores,
    Service,
}

/// The camp: clusters scattered over it (each its own trampled ground, its clutter, its road), a spine road from the
/// road it meets in among them to the farthest, the land's trees standing between them.
fn lay_camp(l: &mut Layout<'_, '_>, camp: &Camp) {
    let (w, d) = (camp.size.x, camp.size.y);
    // The clusters' places (none nearer another than 42 m).
    let count = if l.fighter { 10 } else { 13 };
    let mut centres: Vec<Vec2> = Vec::new();
    for _ in 0..count * 12 {
        if centres.len() >= count {
            break;
        }
        let p = camp.at(l.rng.signed() * 0.46 * w, l.rng.signed() * 0.44 * d);
        if centres.iter().all(|c| c.distance(p) > 46.0)
            && l.off_strips(p) > 16.0
            && l.off_pieces(p) > 20.0
            && l.fair(p, 18.0, 0.15)
        {
            centres.push(p);
        }
    }
    if centres.is_empty() {
        return;
    }
    // Nearest the runway first: the service apron and the motor pool there, the rest by turns.
    centres.sort_by(|a, b| {
        (*a - camp.centre)
            .dot(camp.v)
            .total_cmp(&(*b - camp.centre).dot(camp.v))
    });
    // The spine: from the road it meets, in at the camp's near side, past the clusters on its way (beside them) to
    // the farthest, ending at its edge.
    let entry = camp.at(l.rng.signed() * 0.2 * w, -0.5 * d);
    let mut spine = Vec::new();
    if let Some(q) = l.nearest_road(entry) {
        spine.push(q);
    }
    spine.push(entry);
    let far = *centres.last().expect("a cluster");
    for c in centres.iter().skip(1).step_by(2) {
        if *c != far {
            let beside = if (*c - camp.centre).dot(camp.u) > 0.0 {
                -1.0
            } else {
                1.0
            };
            spine.push(*c + camp.u * (beside * 26.0));
        }
    }
    let last = *spine.last().expect("an entry");
    let end = far + (last - far).normalize_or(-camp.v) * 14.0;
    spine.push(end);
    if !l.road_along(&spine, 3.5, Look::Road) {
        return;
    }
    // Its end: worn ground where trucks turn.
    l.footprint(end, end, 9.0);
    let mut kinds = vec![
        Cluster::Service,
        Cluster::Motor,
        Cluster::Mess,
        Cluster::Stores,
    ];
    kinds.truncate(centres.len().min(4));
    let rest = centres.len().saturating_sub(kinds.len());
    for k in 0..rest {
        kinds.push(if k % 4 == 3 {
            Cluster::Quonsets
        } else {
            Cluster::Tents
        });
    }
    let mut stood: Vec<(Vec2, f32)> = Vec::new();
    for (centre, kind) in centres.into_iter().zip(kinds) {
        let axis = rotate(camp.u, l.rng.signed() * 32.0);
        let reach = match kind {
            Cluster::Tents => tents(l, centre, axis),
            Cluster::Quonsets => quonsets(l, centre, axis),
            Cluster::Mess => mess(l, centre, axis),
            Cluster::Motor => motor_pool(l, centre, axis),
            Cluster::Stores => stores(l, centre, axis),
            Cluster::Service => service(l, centre, axis),
        };
        if reach <= 0.0 {
            continue;
        }
        clutter(l, centre, axis, reach);
        l.join(centre, reach, 2.6);
        stood.push((centre, reach));
    }
    // Worn short cuts between neighbouring clusters.
    for i in 0..stood.len() {
        let (a, ra) = stood[i];
        let near = stood
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .min_by(|x, y| x.1.0.distance(a).total_cmp(&y.1.0.distance(a)));
        if let Some((_, &(b, rb))) = near
            && l.rng.chance(0.5)
        {
            let dir = (b - a).normalize_or_zero();
            let (from, to) = (a + dir * (0.9 * ra), b - dir * (0.9 * rb));
            if from.distance(to) > 12.0 {
                let mid = (from + to) * 0.5 + perp(dir) * l.rng.signed() * 6.0;
                l.road_along(&[from, mid, to], 1.4, Look::Track);
            }
        }
    }
    water_tower(l, camp, &stood);
    poles(l, &spine_line(l, camp));
    vehicles_on_roads(l, camp);
}

/// The spine road as laid (the first road ribbon entering the camp from its near side), for the poles beside it.
fn spine_line(l: &Layout<'_, '_>, camp: &Camp) -> Vec<Vec2> {
    let inside = |p: Vec2| {
        let d = p - camp.centre;
        d.dot(camp.u).abs() < 0.5 * camp.size.x + 20.0
            && d.dot(camp.v).abs() < 0.5 * camp.size.y + 20.0
    };
    l.plan
        .ribbons
        .iter()
        .filter(|r| r.look == Look::Road && r.half > 3.0)
        .filter(|r| r.points.iter().filter(|p| inside(**p)).count() * 2 > r.points.len())
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .map(|r| r.points.clone())
        .unwrap_or_default()
}

/// A water tower above the camp: a tank on a braced timber stand, by its first cluster.
fn water_tower(l: &mut Layout<'_, '_>, camp: &Camp, stood: &[(Vec2, f32)]) {
    for &(centre, reach) in stood.iter().take(4) {
        for k in 0..6 {
            let at = centre + Vec2::from_angle(k as f32 * TAU / 6.0 + 0.4) * (reach + 7.0);
            if l.fair(at, 4.0, 0.1) && l.off_pieces(at) > 4.0 && l.off_strips(at) > 4.0 {
                l.plan.tanks.push(Tank {
                    centre: at,
                    radius: 2.2,
                    stand: l.rng.range(7.0, 9.0),
                    height: 3.0,
                    tower: true,
                });
                let _ = camp;
                return;
            }
        }
    }
}

/// Telephone poles beside the spine road, some 38 m apart, wired.
fn poles(l: &mut Layout<'_, '_>, spine: &[Vec2]) {
    if spine.len() < 2 {
        return;
    }
    let mut line = Vec::new();
    let mut along = 10.0;
    let total: f32 = spine.windows(2).map(|w| w[0].distance(w[1])).sum();
    while along < total {
        let mut left = along;
        for w in spine.windows(2) {
            let length = w[0].distance(w[1]);
            if left <= length {
                let dir = (w[1] - w[0]) / length.max(1e-3);
                let at = w[0] + dir * left + perp(dir) * 6.0;
                if l.dry(at) && l.off_pieces(at) > 1.0 && l.off_strips(at) > 0.5 {
                    line.push((at, l.rng.range(7.0, 8.0)));
                }
                break;
            }
            left -= length;
        }
        along += l.rng.range(34.0, 42.0);
    }
    if line.len() >= 2 {
        l.plan.poles.push(PoleLine { poles: line });
    }
}

/// Trucks and jeeps on the camp's roads, along them.
fn vehicles_on_roads(l: &mut Layout<'_, '_>, camp: &Camp) {
    let roads: Vec<Vec<Vec2>> = l
        .plan
        .ribbons
        .iter()
        .filter(|r| r.look == Look::Road)
        .filter(|r| {
            r.points.iter().any(|p| {
                let d = *p - camp.centre;
                d.dot(camp.u).abs() < 0.5 * camp.size.x && d.dot(camp.v).abs() < 0.5 * camp.size.y
            })
        })
        .map(|r| r.points.clone())
        .collect();
    if roads.is_empty() {
        return;
    }
    for _ in 0..4 + l.rng.pick(4) {
        let road = &roads[l.rng.pick(roads.len())];
        if road.len() < 2 {
            continue;
        }
        let i = 1 + l.rng.pick(road.len() - 1);
        let (a, b) = (road[i - 1], road[i]);
        let at = a.lerp(b, l.rng.unit()) + perp((b - a).normalize_or_zero()) * l.rng.signed() * 0.8;
        let dir = (b - a).normalize_or(camp.u) * if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        if l.off_pieces(at) > 4.0 {
            let kind = match l.rng.pick(3) {
                0 => VehicleKind::Jeep,
                1 => VehicleKind::Carrier,
                _ => VehicleKind::Truck,
            };
            l.plan.vehicles.push(Vehicle {
                centre: at,
                axis: dir,
                kind,
                colour: DRAB[l.rng.pick(DRAB.len())],
            });
        }
    }
}

/// Clutter about a cluster: a jeep, a truck, crates, a few drums, here and there.
fn clutter(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2, reach: f32) {
    let place = |l: &mut Layout<'_, '_>, room: f32| {
        let p = centre + Vec2::from_angle(l.rng.range(0.0, TAU)) * (reach + l.rng.range(-3.0, 1.0));
        (l.fair(p, room, 0.15) && l.off_strips(p) > room && l.off_pieces(p) > room).then_some(p)
    };
    if l.rng.chance(0.45)
        && let Some(p) = place(l, 3.0)
    {
        let kind = if l.rng.chance(0.6) {
            VehicleKind::Jeep
        } else {
            VehicleKind::Truck
        };
        l.plan.vehicles.push(Vehicle {
            centre: p,
            axis: rotate(axis, l.rng.range(0.0, 360.0)),
            kind,
            colour: DRAB[l.rng.pick(DRAB.len())],
        });
    }
    for _ in 0..l.rng.pick(3) {
        if let Some(p) = place(l, 2.0) {
            let turn = l.rng.signed() * 40.0;
            crate_stack(l, p, rotate(axis, turn));
        }
    }
    if l.rng.chance(0.35)
        && let Some(p) = place(l, 2.0)
    {
        l.plan.drums.push(Drums {
            centre: p,
            axis: rotate(axis, l.rng.signed() * 50.0),
            along: 2 + l.rng.pick(3) as u8,
            across: 1 + l.rng.pick(2) as u8,
            tiers: 1,
            seed: l.rng.pick(256) as u8,
        });
    }
}

fn crate_stack(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) {
    l.plan.crates.push(Crates {
        centre,
        axis,
        half: Vec2::new(l.rng.range(0.6, 1.6), l.rng.range(0.5, 1.1)),
        height: l.rng.range(0.5, 1.6),
        colour: CRATE[l.rng.pick(CRATE.len())],
    });
}

/// A company street: two rows of pyramidal tents facing each other across a trodden street with a duckboard down it
/// (a row behind each now and then, back to back), a ward tent across its head, a latrine behind, a sandbag wall at
/// its foot; a few tents askew, gaps here and there. Its reach from the middle (0: none stood).
fn tents(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) -> f32 {
    let across = perp(axis);
    let per = 5 + l.rng.pick(4);
    let pitch = l.rng.range(7.0, 7.8);
    let street = l.rng.range(7.0, 9.0);
    let half = 2.45;
    let colour_base = l.rng.pick(CANVAS.len());
    let length = (per - 1) as f32 * pitch;
    let mut reach: f32 = 0.0;
    let mut rows = vec![0.5 * street + half, -0.5 * street - half];
    for side in [1.0f32, -1.0] {
        if l.rng.chance(0.4) {
            rows.push(side * (0.5 * street + 3.0 * half + 2.5));
        }
    }
    let (head, foot) = (
        centre - axis * (0.5 * length + 4.0),
        centre + axis * (0.5 * length + 4.0),
    );
    for v in rows {
        let mut row: Option<(Vec2, Vec2)> = None;
        for k in 0..per {
            let u = (k as f32 - 0.5 * (per - 1) as f32) * pitch;
            let at =
                centre + axis * u + across * v + Vec2::new(l.rng.signed(), l.rng.signed()) * 0.35;
            let askew = if l.rng.chance(0.12) {
                l.rng.signed() * 12.0
            } else {
                l.rng.signed() * 2.0
            };
            let colour = CANVAS[(colour_base + l.rng.pick(2)) % CANVAS.len()];
            let rolled = l.rng.chance(0.35);
            let big = l.rng.chance(0.12);
            if l.rng.chance(0.06)
                || !l.fair(at, 3.5, 0.2)
                || l.off_pieces(at) < 1.5
                || l.off_roads(at) < 3.5
            {
                continue;
            }
            l.plan.tents.push(Tent {
                centre: at,
                axis: rotate(axis, askew),
                half: if big { 2.9 } else { half },
                wall: 1.5,
                peak: if big { 4.2 } else { 3.7 } + l.rng.signed() * 0.15,
                colour,
                rolled,
            });
            reach = reach.max(at.distance(centre) + 3.5);
            row = Some(row.map_or((at, at), |(first, _)| (first, at)));
        }
        // The row's ground: about its tents only.
        if let Some((first, last)) = row {
            l.footprint(first, last, half * 1.42 + 2.5);
        }
    }
    if reach <= 0.0 {
        return 0.0;
    }
    l.footprint(head, foot, 0.5 * street + 1.0);
    // The street worn to earth, its duckboard down it.
    l.plan.ribbons.push(Ribbon {
        points: vec![head, foot],
        half: 0.5 * street + 1.5,
        paved: 0.5 * street + 1.5,
        look: Look::Trodden,
        surface: Surface::Coral,
        lift: DUST_LIFT,
        across: 2,
    });
    if l.rng.chance(0.75) {
        let shift = across * l.rng.signed() * 0.8;
        l.plan.boards.push(Board {
            a: head + axis * 3.0 + shift,
            b: foot - axis * 3.0 + shift,
            half: 0.4,
        });
    }
    // A ward tent across the street's head.
    let ward = head - axis * 5.0;
    if l.fair(ward, 7.0, 0.15) && l.off_pieces(ward) > 2.0 && l.off_strips(ward) > 3.0 {
        l.plan.huts.push(Hut {
            centre: ward,
            axis: across,
            length: l.rng.range(11.0, 15.0),
            half: 2.45,
            height: 4.3,
            kind: HutKind::Ward,
            colour: CANVAS[colour_base],
            ends: CANVAS[(colour_base + 1) % CANVAS.len()],
        });
        reach = reach.max(ward.distance(centre) + 7.0);
    }
    // A latrine behind a row, a sandbag wall at the street's foot.
    let back = centre
        + across * (if l.rng.chance(0.5) { 1.0 } else { -1.0 }) * (0.5 * street + 2.0 * half + 7.0)
        + axis * l.rng.signed() * 0.3 * length;
    if l.fair(back, 2.5, 0.2) && l.off_pieces(back) > 1.5 && l.off_strips(back) > 2.0 {
        l.plan.huts.push(Hut {
            centre: back,
            axis,
            length: 3.6,
            half: 0.95,
            height: 2.3,
            kind: HutKind::Latrine,
            colour: IRON[l.rng.pick(3)],
            ends: PLANKS[l.rng.pick(PLANKS.len())],
        });
    }
    if l.rng.chance(0.6) {
        let at = foot + axis * 3.0;
        let (a, b) = (
            at - across * l.rng.range(3.0, 5.0),
            at + across * l.rng.range(3.0, 5.0),
        );
        if l.dry(a) && l.dry(b) && l.off_pieces(at) > 1.5 {
            l.plan.walls.push(Wall {
                a,
                b,
                height: l.rng.range(0.9, 1.3),
            });
        }
    }
    reach
}

/// A cluster of Quonset huts side by side, in a row or two.
fn quonsets(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) -> f32 {
    let across = perp(axis);
    let rows = 1 + l.rng.pick(2);
    let per = 2 + l.rng.pick(3);
    let painted = l.rng.chance(0.4);
    let mut reach: f32 = 0.0;
    for r in 0..rows {
        let v = (r as f32 - 0.5 * (rows - 1) as f32) * 19.0;
        for k in 0..per {
            let u = (k as f32 - 0.5 * (per - 1) as f32) * l.rng.range(11.0, 13.5);
            let at =
                centre + axis * u + across * v + Vec2::new(l.rng.signed(), l.rng.signed()) * 0.6;
            let colour = if painted {
                IRON[3 + l.rng.pick(2)]
            } else {
                IRON[l.rng.pick(3)]
            };
            if !l.fair(at, 8.0, 0.12) || l.off_strips(at) < 9.0 || l.off_pieces(at) < 3.0 {
                continue;
            }
            l.plan.huts.push(Hut {
                centre: at,
                axis: rotate(across, l.rng.signed() * 4.0),
                length: 14.6,
                half: 3.05,
                height: 3.3,
                kind: HutKind::Quonset,
                colour,
                ends: PLYWOOD[l.rng.pick(PLYWOOD.len())],
            });
            reach = reach.max(at.distance(centre) + 8.0);
        }
    }
    reach
}

/// A mess: a long frame hall or two, a Quonset beside.
fn mess(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) -> f32 {
    let across = perp(axis);
    let mut reach: f32 = 0.0;
    for k in 0..1 + l.rng.pick(2) {
        let length = l.rng.range(22.0, 32.0);
        let at = centre + across * (k as f32 * 15.0 - 7.0);
        if !l.fair(at, 0.5 * length, 0.1)
            || l.off_strips(at) < 0.5 * length
            || l.off_pieces(at) < 4.0
        {
            continue;
        }
        l.plan.huts.push(Hut {
            centre: at,
            axis: rotate(axis, l.rng.signed() * 3.0),
            length,
            half: 4.5,
            height: 3.0,
            kind: HutKind::Frame,
            colour: IRON[l.rng.pick(3)],
            ends: PLANKS[l.rng.pick(PLANKS.len())],
        });
        reach = reach.max(at.distance(centre) + 0.5 * length);
    }
    // Its water: a tank or two on low stands beside it.
    for k in 0..1 + l.rng.pick(2) {
        let at = centre - axis * (reach + 4.0) + across * (k as f32 * 5.5 - 2.5);
        if l.fair(at, 2.0, 0.15) && l.off_pieces(at) > 1.5 && l.off_strips(at) > 2.0 {
            l.plan.tanks.push(Tank {
                centre: at,
                radius: 1.3,
                stand: 1.8,
                height: 2.0,
                tower: false,
            });
        }
    }
    let hut = centre + axis * (reach + 6.0) + across * l.rng.signed() * 6.0;
    if l.fair(hut, 8.0, 0.12) && l.off_pieces(hut) > 3.0 && l.off_strips(hut) > 9.0 {
        l.plan.huts.push(Hut {
            centre: hut,
            axis: across,
            length: 14.6,
            half: 3.05,
            height: 3.3,
            kind: HutKind::Quonset,
            colour: IRON[l.rng.pick(IRON.len())],
            ends: PLYWOOD[l.rng.pick(PLYWOOD.len())],
        });
        reach = reach.max(hut.distance(centre) + 8.0);
    }
    reach
}

/// A motor pool: a coral lot, trucks and jeeps parked loosely, a Quonset for the shop.
fn motor_pool(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) -> f32 {
    let across = perp(axis);
    let half = Vec2::new(l.rng.range(20.0, 28.0), l.rng.range(14.0, 18.0));
    if !l.fair(centre, half.y, 0.1) {
        return 0.0;
    }
    let ring = rough(&mut l.rng, axis, half.x, half.y, 4.0, 0.06, 16);
    l.plan.pads.push(Pad {
        centre,
        ring,
        look: Look::Hardstand,
        lift: LOT_LIFT,
    });
    for row in [-1.0f32, 1.0] {
        let mut u = -half.x + 4.0;
        while u < half.x - 3.0 {
            let kind = match l.rng.pick(4) {
                0 => VehicleKind::Jeep,
                1 => VehicleKind::Carrier,
                _ => VehicleKind::Truck,
            };
            if !l.rng.chance(0.25) {
                let p = centre + axis * u + across * (row * (half.y - 5.5) + l.rng.signed() * 1.2);
                l.plan.vehicles.push(Vehicle {
                    centre: p,
                    axis: rotate(across * row, l.rng.signed() * 14.0),
                    kind,
                    colour: DRAB[l.rng.pick(DRAB.len())],
                });
            }
            u += l.rng.range(3.8, 5.0);
        }
    }
    let shop = centre + across * (half.y + 9.0);
    if l.fair(shop, 8.0, 0.12) && l.off_pieces(shop) > 2.0 && l.off_strips(shop) > 9.0 {
        l.plan.huts.push(Hut {
            centre: shop,
            axis,
            length: 18.0,
            half: 3.05,
            height: 3.3,
            kind: HutKind::Quonset,
            colour: IRON[l.rng.pick(IRON.len())],
            ends: PLYWOOD[l.rng.pick(PLYWOOD.len())],
        });
    }
    half.length().max(half.y + 17.0)
}

/// Stores: crates stacked in heaps, drums, a Quonset.
fn stores(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) -> f32 {
    let across = perp(axis);
    let mut reach: f32 = 0.0;
    for _ in 0..4 + l.rng.pick(5) {
        let p = centre + axis * l.rng.signed() * 13.0 + across * l.rng.signed() * 9.0;
        let turn = l.rng.signed() * 20.0;
        if l.fair(p, 2.0, 0.15) && l.off_pieces(p) > 1.5 && l.off_strips(p) > 3.0 {
            crate_stack(l, p, rotate(axis, turn));
            reach = reach.max(p.distance(centre) + 2.0);
        }
    }
    for k in 0..2 + l.rng.pick(2) {
        let p = centre + axis * (k as f32 * 6.0 - 6.0) - across * 13.0;
        if l.fair(p, 3.0, 0.12) && l.off_pieces(p) > 1.5 && l.off_strips(p) > 3.0 {
            l.plan.drums.push(Drums {
                centre: p,
                axis,
                along: 4 + l.rng.pick(4) as u8,
                across: 2 + l.rng.pick(2) as u8,
                tiers: 1,
                seed: l.rng.pick(256) as u8,
            });
            reach = reach.max(p.distance(centre) + 3.0);
        }
    }
    let hut = centre + across * 15.0;
    if l.fair(hut, 8.0, 0.12) && l.off_pieces(hut) > 2.0 && l.off_strips(hut) > 9.0 {
        l.plan.huts.push(Hut {
            centre: hut,
            axis,
            length: 14.6,
            half: 3.05,
            height: 3.3,
            kind: HutKind::Quonset,
            colour: IRON[l.rng.pick(IRON.len())],
            ends: PLYWOOD[l.rng.pick(PLYWOOD.len())],
        });
        reach = reach.max(hut.distance(centre) + 8.0);
    }
    reach
}

/// A service apron at the camp's edge nearest the runway: a lot with planes under maintenance and a Quonset shop.
fn service(l: &mut Layout<'_, '_>, centre: Vec2, axis: Vec2) -> f32 {
    let across = perp(axis);
    let half = Vec2::new(l.rng.range(24.0, 32.0), 15.0);
    if !l.fair(centre, half.x, 0.08) {
        return 0.0;
    }
    let ring = rough(&mut l.rng, axis, half.x, half.y, 5.0, 0.05, 16);
    l.plan.pads.push(Pad {
        centre,
        ring,
        look: Look::Hardstand,
        lift: LOT_LIFT,
    });
    let count = 2 + l.rng.pick(2);
    for k in 0..count {
        let u = (k as f32 - 0.5 * (count - 1) as f32) * 2.0 * half.x / count as f32;
        let facing = rotate(-across, l.rng.signed() * 25.0);
        l.park(
            centre + axis * u,
            facing,
            half.x / count as f32 + 1.5,
            LOT_LIFT,
        );
    }
    let shop = centre + across * (half.y + 9.0);
    if l.fair(shop, 8.0, 0.12) && l.off_pieces(shop) > 2.0 && l.off_strips(shop) > 9.0 {
        l.plan.huts.push(Hut {
            centre: shop,
            axis,
            length: 18.0,
            half: 3.05,
            height: 3.3,
            kind: HutKind::Quonset,
            colour: IRON[l.rng.pick(3)],
            ends: PLYWOOD[l.rng.pick(PLYWOOD.len())],
        });
    }
    for _ in 0..1 + l.rng.pick(3) {
        let p = centre + axis * l.rng.signed() * half.x + across * (half.y + 2.0);
        if l.fair(p, 2.0, 0.15) && l.off_pieces(p) > 1.5 && l.off_strips(p) > 2.0 {
            let turn = l.rng.signed() * 30.0;
            crate_stack(l, p, rotate(axis, turn));
        }
    }
    half.length() + 4.0
}

/// The fuel dumps: blocks of standing drums in rows with lanes between, a track to the nearest road.
fn dumps(l: &mut Layout<'_, '_>, camp: &Camp, spots: &mut Vec<Vec2>) {
    let Some(at) = take_spot(l, spots, 45.0) else {
        return;
    };
    let count = if l.fighter { 2 } else { 3 };
    for k in 0..count {
        let centre = at
            + camp.u * (k as f32 - 0.5 * (count - 1) as f32) * 55.0
            + camp.v * l.rng.signed() * 12.0;
        if !l.fair(centre, 22.0, 0.1) || l.off_strips(centre) < 30.0 || l.off_pieces(centre) < 25.0
        {
            continue;
        }
        let cols = 2 + l.rng.pick(3);
        let mut first = None;
        let mut last = centre;
        for i in 0..cols {
            for j in 0..2 {
                let along = 6 + l.rng.pick(5) as u8;
                let across = 4 + l.rng.pick(3) as u8;
                let pu = (i as f32 - 0.5 * (cols - 1) as f32) * 9.0 + l.rng.signed() * 1.5;
                let pv = (j as f32 - 0.5) * 7.5 + l.rng.signed() * 1.0;
                let p = centre + camp.u * pu + camp.v * pv;
                if l.dry(p) {
                    l.plan.drums.push(Drums {
                        centre: p,
                        axis: rotate(camp.u, l.rng.signed() * 6.0),
                        along,
                        across,
                        tiers: if l.rng.chance(0.3) { 2 } else { 1 },
                        seed: l.rng.pick(256) as u8,
                    });
                    first.get_or_insert(p);
                    last = p;
                }
            }
        }
        if let Some(first) = first {
            // Worn ground about the blocks and the lanes between, the trees close about it.
            l.footprint(first, last, 7.0);
            l.join(centre, 18.0, 2.5);
        }
    }
}

/// The heavy battery: four 90 mm pits in a diamond beyond the camp, a track to the nearest road.
fn battery(l: &mut Layout<'_, '_>, camp: &Camp, spots: &mut Vec<Vec2>) {
    let Some(at) = take_spot(l, spots, 30.0) else {
        return;
    };
    let turn = l.rng.range(0.0, 90.0);
    let aim = rotate(camp.v, l.rng.signed() * 40.0);
    for k in 0..4 {
        let dir = rotate(camp.u, turn + 90.0 * k as f32);
        let centre = at + dir * l.rng.range(18.0, 22.0);
        l.plan.pits.push(Pit {
            centre,
            radius: 3.8,
            opening: -dir,
            gun: Gun::Heavy,
            aim,
        });
    }
    // Worn ground about the pits and the paths from the command post between them, the trees left between.
    for k in 0..l.plan.pits.len() {
        let pit = l.plan.pits[k];
        if pit.gun == Gun::Heavy && pit.centre.distance(at) < 30.0 {
            l.footprint(at, pit.centre, 3.0);
            l.footprint(pit.centre, pit.centre, pit.radius + 3.5);
        }
    }
    l.footprint(at, at, 8.0);
    l.join(at, 26.0, 2.5);
}

/// 40 mm pits off the runway's ends and sides.
fn light_pits(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let reach = s.half + s.shoulder + 24.0;
    let places = [
        (-0.5 * s.length - 20.0, reach),
        (-0.5 * s.length - 20.0, -reach),
        (0.5 * s.length + 20.0, reach),
        (0.5 * s.length + 20.0, -reach),
        (l.rng.signed() * 0.3 * s.length, -s.side * reach),
    ];
    for (along, across) in places {
        let centre = s.at(along, across);
        if l.fair(centre, 6.0, 0.15) && l.off_strips(centre) > 6.0 && l.off_pieces(centre) > 6.0 {
            let out = (centre - s.centre).normalize_or(s.dir);
            l.plan.pits.push(Pit {
                centre,
                radius: 2.9,
                opening: -out,
                gun: Gun::Light,
                aim: rotate(out, l.rng.signed() * 50.0),
            });
        }
    }
}

/// Tyre tracks worn across the open ground: between neighbouring loops' far sides, from the far end of the dispersal
/// to the runway's end, from the camp's side to the taxiway's ends.
fn tracks(l: &mut Layout<'_, '_>, taxiway: Taxiway) {
    let s = l.site;
    // The loops' far points (their middles' farthest from the runway).
    let far: Vec<Vec2> = l
        .plan
        .ribbons
        .iter()
        .filter(|r| r.look == Look::Taxiway && r.lift == LOOP_LIFT)
        .filter_map(|r| {
            r.points.iter().copied().max_by(|a, b| {
                (*a - s.centre)
                    .dot(s.right() * s.side)
                    .total_cmp(&(*b - s.centre).dot(s.right() * s.side))
            })
        })
        .collect();
    let mut lines: Vec<(Vec2, Vec2)> = Vec::new();
    for w in far.windows(2) {
        if l.rng.chance(0.6) {
            lines.push((w[0], w[1]));
        }
    }
    if let (Some(first), Some(last)) = (far.first(), far.last()) {
        let [e0, e1] = s.ends();
        lines.push((
            *first,
            e0 + s.side * s.right() * (taxiway.offset.abs() * 0.4),
        ));
        lines.push((
            *last,
            e1 + s.side * s.right() * (taxiway.offset.abs() * 0.4),
        ));
    }
    // Short cuts worn between hardstands and across the open ground to the taxiway and the runway's shoulders.
    let stands: Vec<Vec2> = l
        .plan
        .pads
        .iter()
        .filter(|p| p.look == Look::Hardstand)
        .map(|p| p.centre)
        .collect();
    for k in 0..stands.len() {
        if !l.rng.chance(0.3) {
            continue;
        }
        let a = stands[k];
        let to = if l.rng.chance(0.5) && k + 1 < stands.len() {
            stands[k + 1]
        } else {
            s.at(
                (a - s.centre).dot(s.dir) + l.rng.signed() * 60.0,
                s.side * (s.half + s.shoulder + 6.0),
            )
        };
        lines.push((a, to));
    }
    for (a, b) in lines {
        let length = a.distance(b);
        if length < 30.0 {
            continue;
        }
        let normal = perp((b - a) / length);
        let line: Vec<Vec2> = (0..=6)
            .map(|k| {
                let t = k as f32 / 6.0;
                let sway = if k == 0 || k == 6 {
                    0.0
                } else {
                    l.rng.signed() * 0.05 * length
                };
                a.lerp(b, t) + normal * sway
            })
            .collect();
        l.road_along(&line, 1.6, Look::Track);
    }
}

#[cfg(test)]
mod tests {
    use super::super::site::{fit, tests::coast, tests::creek};
    use super::*;

    fn plan(size: SizeClass, seed: u32) -> Plan {
        let input = SiteInput {
            seed,
            size,
            template: super::super::Template::UsPacificStrip,
            gravel: false,
            heading_hint: None,
            ground: &coast,
            water: &creek,
            others: &[],
            runway: None,
        };
        let site = fit(&input).expect("a site");
        us_pacific_strip(site, &input)
    }

    #[test]
    fn a_base_is_laid_out_alike_every_time_and_on_dry_ground() {
        for size in [SizeClass::FighterStrip, SizeClass::BomberField] {
            let a = plan(size, 41);
            assert_eq!(a, plan(size, 41), "laid out alike");
            assert!(!a.revetments.is_empty(), "{size:?}: revetments");
            assert!(a.stands >= a.revetments.len());
            assert!(a.tents.len() >= 20, "{size:?}: {} tents", a.tents.len());
            assert!(a.huts.len() >= 4, "{size:?}: {} huts", a.huts.len());
            assert_eq!(a.towers.len(), 1);
            assert!(!a.drums.is_empty() && !a.pits.is_empty() && !a.vehicles.is_empty());
            assert!(!a.crates.is_empty(), "{size:?}: crates");
            // Every piece on dry ground and in the clearing (no tree through it).
            let points = a
                .tents
                .iter()
                .map(|t| t.centre)
                .chain(a.huts.iter().map(|h| h.centre))
                .chain(a.revetments.iter().map(|r| r.centre))
                .chain(a.pads.iter().map(|p| p.centre))
                .chain(a.pits.iter().map(|p| p.centre))
                .chain(a.towers.iter().map(|t| t.centre))
                .chain(a.parked.iter().map(|p| p.centre));
            for p in points {
                assert!(
                    !creek(p) && coast(p) >= super::super::site::DRY_M,
                    "{p} wet"
                );
                assert!(a.holds(p), "{p} outside the clearing");
            }
            for r in &a.ribbons {
                for p in &r.points {
                    assert!(!creek(*p), "a strip at {p} in the water");
                    assert!(a.holds(*p), "a strip at {p} outside the clearing");
                }
            }
            // A tent never on a strip.
            for t in &a.tents {
                for r in a
                    .ribbons
                    .iter()
                    .filter(|r| !matches!(r.look, Look::Track | Look::Trodden))
                {
                    assert!(
                        line_distance(t.centre, &r.points) > r.half + t.half,
                        "a tent at {} on a strip",
                        t.centre
                    );
                }
            }
            // Most hardstands hold a plane, of the base's types, off the runway.
            let site = a.site.expect("a site");
            let on_stands = a.parked.len();
            assert!(
                on_stands >= a.stands / 2,
                "{size:?}: {on_stands} planes, {} stands",
                a.stands
            );
            for p in &a.parked {
                assert!(
                    FIGHTERS
                        .iter()
                        .chain(&BOMBERS)
                        .any(|(id, _)| *id == p.model)
                );
                let across = (p.centre - site.centre).dot(site.right()).abs();
                assert!(
                    across > site.half + site.shoulder + 5.0,
                    "a plane on the runway at {}",
                    p.centre
                );
            }
            let headings: Vec<i32> = a.parked.iter().map(|p| (p.heading / 30.0) as i32).collect();
            assert!(
                headings.iter().any(|h| *h != headings[0]),
                "{size:?}: every plane facing one way"
            );
            // Every road joins another, a taxiway or what it serves: its ends near a strip, a pad or a piece.
            for r in a.ribbons.iter().filter(|r| r.look == Look::Road) {
                for end in [r.points[0], *r.points.last().expect("points")] {
                    let joined = a
                        .ribbons
                        .iter()
                        .filter(|o| !std::ptr::eq(*o, r) && o.look != Look::Track)
                        .any(|o| line_distance(end, &o.points) < o.half + 8.0)
                        || a.pads
                            .iter()
                            .any(|p| p.centre.distance(end) < p.reach() + 8.0)
                        || a.tents
                            .iter()
                            .map(|t| t.centre)
                            .chain(a.drums.iter().map(|d| d.centre))
                            .chain(a.pits.iter().map(|p| p.centre))
                            .chain(a.crates.iter().map(|c| c.centre))
                            .any(|c| c.distance(end) < 14.0)
                        || a.huts
                            .iter()
                            .any(|h| h.centre.distance(end) - 0.5 * h.length < 12.0);
                    assert!(joined, "{size:?}: a road ends nowhere at {end}");
                }
            }
        }
        // Another seed lays it out otherwise.
        assert_ne!(
            plan(SizeClass::BomberField, 41).tents,
            plan(SizeClass::BomberField, 42).tents
        );
    }

    #[test]
    fn smoothing_keeps_the_ends() {
        let line = [Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)];
        let s = smooth(&line, 3);
        assert_eq!(s.first(), Some(&line[0]));
        assert_eq!(s.last(), Some(&line[2]));
        assert!(s.len() > 6);
    }
}
