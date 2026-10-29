//! Latitude–longitude squares and the footprints that meet them (globe stage 2, step C: C1, C2). Inert: nothing calls
//! it yet. LAND turns a need set's caps and a query's footprint (`land::Footprint`) into the squares it admits.
//!
//! **A square** ([`Square`]) is 1° × 1°, named by its south-west corner in whole degrees: latitude −90 to 89,
//! longitude −180 to 179, so no square straddles ±180°. Squares are ordered by latitude, then longitude: the order of
//! `naval-land-format`'s `SquareKey`, of a release's square table and of an admission. A place belongs to one square,
//! half-open ([`Square::at`]); a square meeting the pole limit (89.5°) is [`Square::polar`], open sea in every release.
//!
//! **Covers.** [`cap`], [`corridor`] and [`curve`] give every square a footprint meets, in key order. A square is
//! closed: a footprint that only touches its edge or its corner takes it, so whichever square owns a place on that
//! edge is admitted, across ±180° and over a pole alike. Each cover is conservative, never missing a square, and
//! allows [`COVER_SLACK_M`] for its own rounding. The radius or half-width given is the footprint's, its
//! interpolation's support included ("plus one sample", C2: `land::Land::footprint` adds it).
//!
//! - [`cap`]: exact. A square is taken when its nearest point lies within the radius of the centre: the latitude gap
//!   when the centre's longitude is the square's, else the nearer of its two meridian edges (across to the edge, or
//!   to its nearer corner).
//! - [`corridor`], a geodesic segment's: the caps of `w + h/2` about samples of the great-circle arc at most
//!   [`CORRIDOR_SAMPLE_M`] (`h`) apart, its ends included. Every place within `w` of the arc lies within `w + h/2` of a
//!   sample, so nothing is missed; a square at most `h/2` (500 m) beyond the corridor may be taken too. Two places
//!   within `sphere::ANTIPODE_GUARD_M` of antipodal have no one great circle, and take every square.
//! - [`curve`], a chart curve's (C2): the curve given by its samples as places, at most [`CURVE_SAMPLE_M`] apart. Between
//!   two samples it leaves their great-circle chord by at most [`CURVE_CHORD_SLACK_M`] inside the far edge, so it is
//!   the union of the chords' corridors, each widened by that much. Never the great circle between the curve's ends.
//!
//! **The test.** A candidate square is taken when `gap(c, sq) <= ρ`. [`within`] answers it without `gap`'s `atan2`:
//! the sines and cosines of whole degrees come from a table, bit for bit those `gap` takes, and the comparisons are
//! products of them. Where a comparison lies within [`TIE`] of its threshold it asks `gap` itself, so its answer is
//! `gap`'s in every case, to the bit (`squares_tests.rs` holds it to `gap` at random radii and at ties).
//!
//! **One set** ([`Cover`]): every footprint of a need set covered into one set of the Earth's squares, a bit each, a
//! candidate another footprint took never tested again. Its squares are the union of each footprint's own cover.
//!
//! **A kept cover** ([`Kept`]): a cap's squares, and how far its centre may move and its radius change before one
//! could join or leave them: the distance from its edge to the nearest square edge it crosses, or [`Kept::cap`]'s
//! reach when that is nearer. Within it the squares are exactly those a fresh cover gives ([`Kept::holds`]).
use super::sphere::{R, is_antipodal};
use crate::vector::{add, cross, dot, scale, sub};
use std::collections::BTreeSet;
use std::f64::consts::PI;
use std::sync::OnceLock;

/// A chart curve's footprint samples it every this many metres of its length, as places (the land's curve family,
/// `naval_sim::land`, re-exports it).
pub const CURVE_SAMPLE_M: f64 = 1_000.0;
/// How far a chart curve leaves the chord between two of its samples, at most: §3.5's 10.077 km bow of a 300 km leg at
/// the far edge scaled by (1/300)², 0.112 m, rounded up (C2).
pub const CURVE_CHORD_SLACK_M: f64 = 0.12;

type V3 = [f64; 3];

/// What a cover allows for its own rounding, metres: its arithmetic is good to nanometres.
pub const COVER_SLACK_M: f64 = 1e-3;
/// The most a geodesic corridor's samples lie apart ([`corridor`]).
pub const CORRIDOR_SAMPLE_M: f64 = 1_000.0;
/// The pole limit, degrees (`sphere::POLE_LIMIT_Z` is its sine): a square reaching past it is [`Square::polar`].
pub const POLE_LIMIT_DEG: f64 = 89.5;
/// How far a candidate range of rows or columns is widened, degrees, before the exact test: past the rounding of the
/// centre's latitude and longitude.
const WIDEN_DEG: f64 = 1e-9;
/// How near its threshold a comparison of [`within`] may lie before it asks `gap` instead: far past the rounding of
/// either (about 1e-16), 6 µm on the ground.
const TIE: f64 = 1e-12;
/// The largest cap, radians (6,371 km), [`within`] answers by its products; a larger one asks `gap` (the products
/// compare tangents, which hold short of a quarter of the girth).
const PRODUCTS_MAX: f64 = 1.0;
/// What a kept cover's hold allows for rounding, metres: its arithmetic is good to nanometres.
pub const KEEP_SLACK_M: f64 = 1e-3;

/// A place on the Earth: its Earth-fixed unit vector (x toward 0° N 0° E, y toward 0° N 90° E, z north) with its
/// latitude and its longitude in [−180, 180), degrees. `WorldChart::geo` makes one of a place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    v: V3,
    lat: f64,
    lon: f64,
}

impl Geo {
    /// The place an Earth-fixed unit vector names. Two `atan2`.
    #[inline]
    pub fn of(v: V3) -> Self {
        Self {
            v,
            lat: v[2].atan2(v[0].hypot(v[1])).to_degrees(),
            lon: wrap(v[1].atan2(v[0]).to_degrees()),
        }
    }
    /// The place at latitude `lat` and longitude `lon`, degrees (the longitude wrapped). Two `sin` and two `cos`.
    #[inline]
    pub fn degrees(lat: f64, lon: f64) -> Self {
        let lon = wrap(lon);
        let (sp, cp) = lat.to_radians().sin_cos();
        let (sl, cl) = lon.to_radians().sin_cos();
        Self {
            v: [cp * cl, cp * sl, sp],
            lat,
            lon,
        }
    }
    /// The Earth-fixed unit vector.
    #[inline]
    pub fn vector(&self) -> V3 {
        self.v
    }
    /// The latitude, degrees.
    #[inline]
    pub fn lat(&self) -> f64 {
        self.lat
    }
    /// The longitude, degrees, in [−180, 180).
    #[inline]
    pub fn lon(&self) -> f64 {
        self.lon
    }
}

/// A longitude, degrees, in [−180, 180).
#[inline]
fn wrap(lon: f64) -> f64 {
    let w = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if w >= 180.0 { -180.0 } else { w }
}

/// A 1° square by its south-west corner: latitude −90 to 89, longitude −180 to 179. Ordered by latitude, then
/// longitude: a release's square table's order and an admission's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Square {
    pub lat: i32,
    pub lon: i32,
}

impl Square {
    /// Squares on the Earth.
    pub const COUNT: usize = 180 * 360;

    /// The square holding the place at latitude `lat` and longitude `lon`, degrees, the longitude wrapped: half-open,
    /// `[lat, lat + 1) × [lon, lon + 1)`; latitude 90 is in the northernmost row.
    #[inline]
    pub fn at(lat: f64, lon: f64) -> Self {
        Self {
            lat: lat.floor().clamp(-90.0, 89.0) as i32,
            lon: (((lon + 180.0).rem_euclid(360.0).floor() as i32).min(359)) - 180,
        }
    }
    /// The square holding the place `g`.
    #[inline]
    pub fn of(g: &Geo) -> Self {
        Self::at(g.lat, g.lon)
    }
    /// Whether it names a square of the world.
    #[inline]
    pub fn valid(self) -> bool {
        (-90..=89).contains(&self.lat) && (-180..=179).contains(&self.lon)
    }
    /// Whether it meets the pole limit: its north edge past 89.5° N or its south edge past 89.5° S (a release's
    /// `polar`, open sea).
    #[inline]
    pub fn polar(self) -> bool {
        f64::from(self.lat + 1) > POLE_LIMIT_DEG || f64::from(self.lat) < -POLE_LIMIT_DEG
    }
    /// Its place in a square table: south to north, west to east.
    #[inline]
    pub fn index(self) -> usize {
        debug_assert!(self.valid(), "{self:?} is not a square");
        ((self.lat + 90) * 360 + (self.lon + 180)) as usize
    }
    /// The square at a place in a square table.
    #[inline]
    pub fn from_index(index: usize) -> Self {
        assert!(index < Self::COUNT, "square {index} is past the table");
        Self {
            lat: (index / 360) as i32 - 90,
            lon: (index % 360) as i32 - 180,
        }
    }
    /// Every square, in key order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::COUNT).map(Self::from_index)
    }
}

// --- Covers ---

/// The squares meeting the closed cap of `r` ground metres about `centre`, in key order. Exact, with
/// [`COVER_SLACK_M`]. A negative radius meets nothing; one of half the girth or more, everything.
pub fn cap(centre: &Geo, r: f64) -> Vec<Square> {
    let mut out = BTreeSet::new();
    cap_into(centre, r, &mut out);
    out.into_iter().collect()
}

/// The squares meeting the corridor of `half_width` ground metres about the great-circle arc from `a` to `b`, in key
/// order: caps about the arc's samples (the module's note). Every square when the two are within
/// `sphere::ANTIPODE_GUARD_M` of antipodal.
pub fn corridor(a: &Geo, b: &Geo, half_width: f64) -> Vec<Square> {
    let mut out = BTreeSet::new();
    corridor_into(a, b, half_width, &mut out);
    out.into_iter().collect()
}

/// The squares meeting the corridor of `half_width` ground metres about a chart curve given by its samples as places,
/// at most [`CURVE_SAMPLE_M`] apart, in key order: each chord's corridor, widened by [`CURVE_CHORD_SLACK_M`]. One
/// sample is a cap; none, nothing.
pub fn curve(samples: &[Geo], half_width: f64) -> Vec<Square> {
    let mut out = BTreeSet::new();
    curve_into(samples, half_width, &mut out);
    out.into_iter().collect()
}

/// Where a cover puts its squares: a few in a [`BTreeSet`], or the one set of a need set ([`Cover`]).
trait Sink {
    fn has(&self, sq: Square) -> bool;
    fn put(&mut self, sq: Square);
    /// Every square.
    fn fill(&mut self);
}

impl Sink for BTreeSet<Square> {
    #[inline]
    fn has(&self, sq: Square) -> bool {
        self.contains(&sq)
    }
    #[inline]
    fn put(&mut self, sq: Square) {
        self.insert(sq);
    }
    fn fill(&mut self) {
        self.extend(Square::all());
    }
}

/// The words of a [`Cover`]: a bit for each square.
const WORDS: usize = Square::COUNT.div_ceil(64);

/// A set of the Earth's squares, a bit each, iterated in key order: what a need set's footprints are covered into
/// together ([`Cover::cap`], [`Cover::corridor`], [`Cover::curve`]), each candidate another took skipped. Its squares
/// are the union of the footprints' own covers ([`cap`], [`corridor`], [`curve`]). Clearing it costs the words it
/// touched.
#[derive(Clone, Debug)]
pub struct Cover {
    bits: Vec<u64>,
    /// The words that may hold a bit: `lo..hi`.
    lo: usize,
    hi: usize,
}

impl Default for Cover {
    fn default() -> Self {
        Self {
            bits: vec![0; WORDS],
            lo: WORDS,
            hi: 0,
        }
    }
}

impl Cover {
    pub fn new() -> Self {
        Self::default()
    }
    /// Empty again.
    pub fn clear(&mut self) {
        if self.lo < self.hi {
            self.bits[self.lo..self.hi].fill(0);
        }
        (self.lo, self.hi) = (WORDS, 0);
    }
    #[inline]
    pub fn contains(&self, sq: Square) -> bool {
        let i = sq.index();
        (self.bits[i >> 6] >> (i & 63)) & 1 != 0
    }
    #[inline]
    pub fn insert(&mut self, sq: Square) {
        let i = sq.index();
        let w = i >> 6;
        self.bits[w] |= 1 << (i & 63);
        self.lo = self.lo.min(w);
        self.hi = self.hi.max(w + 1);
    }
    /// The squares of `other`, and no others.
    pub fn assign(&mut self, other: &Cover) {
        self.clear();
        if other.lo < other.hi {
            self.bits[other.lo..other.hi].copy_from_slice(&other.bits[other.lo..other.hi]);
        }
        (self.lo, self.hi) = (other.lo, other.hi);
    }
    /// Every square of `squares`.
    pub fn extend(&mut self, squares: &[Square]) {
        for &sq in squares {
            self.insert(sq);
        }
    }
    pub fn len(&self) -> usize {
        self.bits[self.lo.min(self.hi)..self.hi]
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Its squares, in key order.
    pub fn iter(&self) -> impl Iterator<Item = Square> + '_ {
        (self.lo.min(self.hi)..self.hi).flat_map(move |w| {
            let mut word = self.bits[w];
            std::iter::from_fn(move || {
                (word != 0).then(|| {
                    let b = word.trailing_zeros() as usize;
                    word &= word - 1;
                    Square::from_index(w * 64 + b)
                })
            })
        })
    }
    /// Adds [`cap`]`(centre, r)`.
    pub fn cap(&mut self, centre: &Geo, r: f64) {
        cap_into(centre, r, self);
    }
    /// Adds [`corridor`]`(a, b, half_width)`.
    pub fn corridor(&mut self, a: &Geo, b: &Geo, half_width: f64) {
        corridor_into(a, b, half_width, self);
    }
    /// Adds [`curve`]`(samples, half_width)`.
    pub fn curve(&mut self, samples: &[Geo], half_width: f64) {
        curve_into(samples, half_width, self);
    }
}

impl Sink for Cover {
    #[inline]
    fn has(&self, sq: Square) -> bool {
        self.contains(sq)
    }
    #[inline]
    fn put(&mut self, sq: Square) {
        self.insert(sq);
    }
    fn fill(&mut self) {
        self.bits.fill(u64::MAX);
        let tail = Square::COUNT % 64;
        if tail != 0 {
            self.bits[WORDS - 1] = (1 << tail) - 1;
        }
        (self.lo, self.hi) = (0, WORDS);
    }
}

/// A cap's candidate squares: its rows, and its columns from `west` (unwrapped) `span` of them.
#[derive(Clone, Copy)]
struct Candidates {
    south: i32,
    north: i32,
    west: i32,
    span: i32,
}

impl Candidates {
    /// Rows: every latitude within ρ. Columns: every longitude when the cap holds a pole, else those between the two
    /// meridians tangent to it, `asin(sin ρ / cos φ)` either side. Both are widened past rounding; the exact test
    /// decides.
    fn of(c: &Geo, rho: f64) -> Self {
        let reach = rho.to_degrees() + WIDEN_DEG;
        let south = (c.lat - reach).floor().max(-90.0) as i32;
        let north = (c.lat + reach).floor().min(89.0) as i32;
        let half = if 90.0 - c.lat.abs() <= reach {
            180.0
        } else {
            (rho.sin() / c.lat.to_radians().cos())
                .min(1.0)
                .asin()
                .to_degrees()
                + WIDEN_DEG
        };
        let (west, span) = if half >= 179.0 {
            (-180, 360)
        } else {
            let west = (c.lon - half).floor() as i32;
            let east = (c.lon + half).floor() as i32;
            (west, (east - west + 1).min(360))
        };
        Self {
            south,
            north,
            west,
            span,
        }
    }
    /// Every candidate, row by row; within a row from the west, across ±180° where the cap does.
    #[inline]
    fn each(self) -> impl Iterator<Item = Square> {
        (self.south..=self.north).flat_map(move |lat| {
            (0..self.span).map(move |k| Square {
                lat,
                lon: column(self.west + k),
            })
        })
    }
}

fn cap_into<S: Sink>(c: &Geo, r: f64, out: &mut S) {
    if r.is_nan() || r < 0.0 {
        return;
    }
    let rho = (r + COVER_SLACK_M) / R;
    if rho >= PI {
        out.fill();
        return;
    }
    let range = Candidates::of(c, rho);
    if range.south == range.north && range.span == 1 {
        out.put(Square {
            lat: range.south,
            lon: column(range.west),
        });
        return;
    }
    let probe = Probe::new(rho);
    for sq in range.each() {
        if !out.has(sq) && within(c, sq, &probe) {
            out.put(sq);
        }
    }
}

fn curve_into<S: Sink>(samples: &[Geo], half_width: f64, out: &mut S) {
    match samples {
        [] => {}
        [one] => cap_into(one, half_width, out),
        _ => {
            for w in samples.windows(2) {
                debug_assert!(
                    angle(w[0].v, w[1].v) * R <= CURVE_SAMPLE_M * (1.0 + 1e-9),
                    "a chart curve's samples lie {} m apart, past {CURVE_SAMPLE_M} m",
                    angle(w[0].v, w[1].v) * R
                );
                corridor_into(&w[0], &w[1], half_width + CURVE_CHORD_SLACK_M, out);
            }
        }
    }
}

// --- Kept covers ---

/// A cap's cover with how far it holds (the module's note): kept from one boundary to the next for a body that moves
/// little between them (C2's need set, `land::KeptCovers`).
#[derive(Clone, Debug)]
pub struct Kept {
    /// The centre, Earth-fixed, and the radius, ground metres.
    v: V3,
    r: f64,
    /// How far the centre may move plus how much the radius may change, ground metres, with the squares the same.
    /// Negative: it holds nowhere.
    hold_m: f64,
    squares: Vec<Square>,
}

impl Kept {
    /// [`cap`]`(c, r)`, with its hold: the least distance from the cap's edge to any square's nearest point, either
    /// way, or `reach` metres when every square lies farther, less [`KEEP_SLACK_M`]. Each candidate within the cap
    /// grown by `reach` is tested at the cap shrunk and grown by `reach` ([`within`]); only one that lies between
    /// takes `gap` itself. A cap past [`PRODUCTS_MAX`] or with no reach is covered and holds nowhere.
    pub fn cap(c: &Geo, r: f64, reach: f64) -> Self {
        let rho = (r + COVER_SLACK_M) / R;
        let wide = (r + reach + COVER_SLACK_M) / R;
        if !(r >= 0.0 && reach > 0.0 && wide <= PRODUCTS_MAX) {
            return Self {
                v: c.v,
                r,
                hold_m: -1.0,
                squares: cap(c, r),
            };
        }
        let (inner, outer) = (
            Probe::new((r - reach + COVER_SLACK_M) / R),
            Probe::new(wide),
        );
        let mut margin = reach / R;
        let mut squares = Vec::new();
        for sq in Candidates::of(c, wide).each() {
            if !within(c, sq, &outer) {
                continue;
            }
            if within(c, sq, &inner) {
                squares.push(sq);
                continue;
            }
            let g = gap(c, sq);
            if g <= rho {
                squares.push(sq);
            }
            margin = margin.min((g - rho).abs());
        }
        squares.sort_unstable();
        Self {
            v: c.v,
            r,
            hold_m: margin * R - KEEP_SLACK_M,
            squares,
        }
    }
    /// Whether the cap of `r` ground metres about the Earth-fixed unit vector `v` meets exactly these squares: its
    /// centre moved and its radius changed by no more than the hold in all. No square's gap to the centre changes by
    /// more than the centre moved, so none crosses the edge.
    #[inline]
    pub fn holds(&self, v: V3, r: f64) -> bool {
        angle(self.v, v) * R + (r - self.r).abs() <= self.hold_m
    }
    /// The squares, in key order.
    pub fn squares(&self) -> &[Square] {
        &self.squares
    }
    /// How far it holds, ground metres; negative where it holds nowhere.
    pub fn hold_m(&self) -> f64 {
        self.hold_m
    }
}

fn corridor_into<S: Sink>(a: &Geo, b: &Geo, w: f64, out: &mut S) {
    if w.is_nan() || w < 0.0 {
        return;
    }
    if a.v != b.v && is_antipodal(a.v, b.v) {
        out.fill();
        return;
    }
    let len = angle(a.v, b.v);
    let n = ((len * R / CORRIDOR_SAMPLE_M).ceil() as usize).max(1);
    let step = len / n as f64;
    let reach = w + 0.5 * step * R;
    cap_into(a, reach, out);
    cap_into(b, reach, out);
    if n == 1 {
        return;
    }
    // The unit tangent at `a` toward `b`: `b − (a·b)·a`, past a quarter of the girth `s − (a·s)·a` with `s = a + b`,
    // which does not cancel near the antipode.
    let ab = dot(a.v, b.v);
    let t = if ab >= 0.0 {
        sub(b.v, scale(a.v, ab))
    } else {
        let s = add(a.v, b.v);
        sub(s, scale(a.v, dot(a.v, s)))
    };
    let t = scale(t, 1.0 / dot(t, t).sqrt());
    for k in 1..n {
        let (sin, cos) = (k as f64 * step).sin_cos();
        cap_into(&Geo::of(add(scale(a.v, cos), scale(t, sin))), reach, out);
    }
}

/// A column's longitude, −180 to 179.
#[inline]
fn column(lon: i32) -> i32 {
    (lon + 180).rem_euclid(360) - 180
}

/// The sine and cosine of the whole degree `d`, −180 to 180: `f64::from(d).to_radians().sin_cos()`, from a table.
#[inline]
fn whole(d: i32) -> (f64, f64) {
    static TABLE: OnceLock<Vec<(f64, f64)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (-180..=180)
            .map(|d| f64::from(d).to_radians().sin_cos())
            .collect()
    })[(d + 180) as usize]
}

/// `lon − west` in [0, 360): `rem_euclid`'s value, by a comparison where the difference is within a turn.
#[inline]
fn east_of(lon: f64, west: f64) -> f64 {
    let d = lon - west;
    if (0.0..360.0).contains(&d) {
        d
    } else if (-360.0..0.0).contains(&d) {
        d + 360.0
    } else {
        d.rem_euclid(360.0)
    }
}

/// The angle from `c` to the nearest point of the closed square `sq`, radians: the latitude gap when `c`'s longitude
/// is the square's, else the nearer of its two meridian edges.
fn gap(c: &Geo, sq: Square) -> f64 {
    let (south, north) = (f64::from(sq.lat), f64::from(sq.lat + 1));
    if east_of(c.lon, f64::from(sq.lon)) <= 1.0 {
        return (south - c.lat).max(c.lat - north).max(0.0).to_radians();
    }
    meridian_gap(c, sq.lon, sq.lat).min(meridian_gap(c, sq.lon + 1, sq.lat))
}

/// The angle from `c` to the meridian edge at longitude `lon` between latitudes `lat` and `lat + 1`, radians: across
/// to it when the foot of the perpendicular to its great circle lies on it, else to its nearer end.
fn meridian_gap(c: &Geo, lon: i32, lat: i32) -> f64 {
    let (south, north) = (f64::from(lat), f64::from(lat + 1));
    let (s, co) = whole(lon);
    let v = c.v;
    // On the meridian's half-plane and across it.
    let along = v[0] * co + v[1] * s;
    let across = v[1] * co - v[0] * s;
    if along >= 0.0 {
        let foot = v[2].atan2(along).to_degrees();
        if (south..=north).contains(&foot) {
            // The angle to the meridian's plane, `atan2` rather than `asin`, which loses its precision near 90°.
            return across.abs().atan2(along.hypot(v[2]));
        }
    }
    let corner = |lat: i32| {
        let (sp, cp) = whole(lat);
        [cp * co, cp * s, sp]
    };
    angle(v, corner(lat)).min(angle(v, corner(lat + 1)))
}

/// A cap's threshold for [`within`]: ρ, its sine and cosine, and whether the products answer for it.
#[derive(Clone, Copy)]
struct Probe {
    rho: f64,
    sin: f64,
    cos: f64,
    products: bool,
}

impl Probe {
    fn new(rho: f64) -> Self {
        let (sin, cos) = rho.sin_cos();
        Self {
            rho,
            sin,
            cos,
            products: (0.0..=PRODUCTS_MAX).contains(&rho),
        }
    }
}

/// Whether the closed square `sq` meets the cap of `p.rho` about `c`: `gap(c, sq) <= ρ`, always `gap`'s answer (the
/// module's note). The latitude gap is `gap`'s own; a meridian edge is answered by [`meridian_within`], or by
/// `meridian_gap` where that ties.
#[inline]
fn within(c: &Geo, sq: Square, p: &Probe) -> bool {
    if !p.products {
        return gap(c, sq) <= p.rho;
    }
    if east_of(c.lon, f64::from(sq.lon)) <= 1.0 {
        let (south, north) = (f64::from(sq.lat), f64::from(sq.lat + 1));
        return (south - c.lat).max(c.lat - north).max(0.0).to_radians() <= p.rho;
    }
    // `gap` is the nearer edge's: within when either is.
    let edge = |lon: i32| {
        meridian_within(c, lon, sq.lat, p).unwrap_or_else(|| meridian_gap(c, lon, sq.lat) <= p.rho)
    };
    edge(sq.lon) || edge(sq.lon + 1)
}

/// `meridian_gap(c, lon, lat) <= ρ` by products of sines and cosines, or `None` where a comparison lies within [`TIE`]
/// of its threshold or the place lies far off the meridian's plane. With `h = hypot(along, v₂)` and the foot at
/// latitude `φ`: `v₂·cos S − along·sin S = h·sin(φ − S)`, so the foot is north of the south end when it is positive
/// and south of the north end when `v₂·cos N − along·sin N` is negative; across, `atan2(|across|, h) <= ρ` when
/// `|across|·cos ρ − h·sin ρ <= 0`; to an end, its angle is within ρ when `v·end >= cos ρ`.
#[inline]
fn meridian_within(c: &Geo, lon: i32, lat: i32, p: &Probe) -> Option<bool> {
    let (s, co) = whole(lon);
    let v = c.v;
    let along = v[0] * co + v[1] * s;
    let across = v[1] * co - v[0] * s;
    let (ss, cs) = whole(lat);
    let (sn, cn) = whole(lat + 1);
    if along >= 0.0 {
        let h2 = along * along + v[2] * v[2];
        if h2 < 0.25 {
            return None;
        }
        let north_of_south = v[2] * cs - along * ss;
        let south_of_north = v[2] * cn - along * sn;
        if north_of_south.abs() <= TIE || south_of_north.abs() <= TIE {
            return None;
        }
        if north_of_south > 0.0 && south_of_north < 0.0 {
            let d = across.abs() * p.cos - h2.sqrt() * p.sin;
            if d.abs() <= TIE {
                return None;
            }
            return Some(d <= 0.0);
        }
    }
    let to_end = |sp: f64, cp: f64| v[0] * cp * co + v[1] * cp * s + v[2] * sp - p.cos;
    let (south, north) = (to_end(ss, cs), to_end(sn, cn));
    if south > TIE || north > TIE {
        Some(true)
    } else if south < -TIE && north < -TIE {
        Some(false)
    } else {
        None
    }
}

/// The angle between two unit vectors, radians: `atan2(|a × b|, a·b)`, good to rounding at every angle.
#[inline]
pub(super) fn angle(a: V3, b: V3) -> f64 {
    let x = cross(a, b);
    dot(x, x).sqrt().atan2(dot(a, b))
}

#[cfg(test)]
#[path = "squares_tests.rs"]
mod tests;
