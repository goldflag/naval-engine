//! Synthetic sources for the bake, with no download (BAKE-1's proofs, and fixtures for the lanes after it): a generated
//! surveyed layer and generated water polygons, deterministic.
//!
//! - **A [`World`]** is a list of [`Scene`]s, each a box of latitude and longitude (longitude continuous about it) with
//!   its sea: one water polygon whose outer ring is the box, a side of it replaced by a [`Mainland`]'s coast when the
//!   scene has one, and whose holes are its [`Feature`]s' outlines; and the water inside the land (an atoll's lagoon, a
//!   pond) as polygons of their own. Outside every scene the world is land, a plateau.
//! - **The DEM** is a function of the place: each feature's own shape with its coast a little off the polygon's (so
//!   the slide and the cut have work), noise, and spikes (so the despike has); and past 60° N and 56° S a stand-in for
//!   Copernicus GLO-30 (other heights on land, other spikes), so the DEM's change there is a change by place.
//! - **[`SyntheticSources`]** serves both to the bake, baking each surveyed square from the DEM when it is first asked
//!   for (sampled on its band's grid and despiked with its halo, as a world square is) and keeping it.
//!
//! [`Scene::across`] lays features along the edge two squares share, so every kind of operation acts on that edge:
//! islands larger than a square's halo cut by it, specks on it, spits rounding sinks (shorter and longer than the
//! component cap), atolls whose lagoons are under and over the cap, ponds in an island, low islands, coasts the DEM puts
//! inside and outside the polygon's, and a mainland's coast crossing it.
//!
//! Everything is `+ − × ÷` and square roots ([`sin_cos`]), so the sources are the same on every target.
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};
use std::sync::{Arc, Mutex};

use naval_land_format::SquareKey;

use super::Halo;
use super::coverage::{GeoPolygon, Water};
use super::lattice::{ROW_M, sin_cos};
use super::surveyed::Surveyed;
use crate::square::{self, Square, SquareSource};
use crate::{BakeError, Dem, SRTM_NORTH, SRTM_SOUTH};

/// Metres a degree of latitude on the theatre's sphere.
pub const M_PER_DEGREE: f64 = ROW_M * 1600.0;
/// The world outside every scene: land this high.
pub const PLATEAU_M: f64 = 40.0;
/// A scene's sea floor away from its land.
pub const FLOOR_M: f64 = -30.0;
/// Spikes: one in this many cells of [`SPIKE_CELL_DEG`], this high and this wide.
const SPIKE_ONE_IN: u64 = 5;
const SPIKE_CELL_DEG: f64 = 0.02;
const SPIKE_M: f64 = 450.0;
const SPIKE_RADIUS_M: f64 = 30.0;
/// The DEM's land stands at least this high inside its coast (over the land threshold).
const COAST_TOP_M: f64 = 0.6;

fn degrees(x: f64) -> (f64, f64) {
    sin_cos(x * PI / 180.0)
}

/// A local frame about a centre: x east and y north, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Local {
    lat: f64,
    lon: f64,
    m_lon: f64,
}

impl Local {
    fn new(lat: f64, lon: f64) -> Self {
        Self {
            lat,
            lon,
            m_lon: M_PER_DEGREE * degrees(lat).1,
        }
    }
    fn xy(&self, lat: f64, lon: f64) -> [f64; 2] {
        [
            (lon - self.lon) * self.m_lon,
            (lat - self.lat) * M_PER_DEGREE,
        ]
    }
    fn place(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [self.lat + y / M_PER_DEGREE, self.lon + x / self.m_lon]
    }
}

/// A feature's shape, in its local metres.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Radius `radius · (1 + Σ a cos(kθ + φ))` for k = 2, 3, 5 and `waves` the (a, φ).
    Blob { radius: f64, waves: [(f64, f64); 3] },
    /// Within `half_width` of the segment `half_length` either side of the centre, along `bearing_deg` (from east).
    Spit {
        half_length: f64,
        half_width: f64,
        bearing_deg: f64,
    },
    /// Within `width` of the circle of `radius`; the lagoon inside is the sea's.
    Atoll { radius: f64, width: f64 },
}

fn circle(radius: f64, n: usize) -> Vec<[f64; 2]> {
    (0..n)
        .map(|k| {
            let (s, c) = sin_cos(TAU * k as f64 / n as f64);
            [radius * c, radius * s]
        })
        .collect()
}

impl Shape {
    /// The radius of a disc about the centre holding it.
    pub fn extent(&self) -> f64 {
        match *self {
            Shape::Blob { radius, waves } => {
                radius * (1.0 + waves.iter().map(|w| w.0.abs()).sum::<f64>())
            }
            Shape::Spit {
                half_length,
                half_width,
                ..
            } => half_length + half_width,
            Shape::Atoll { radius, width } => radius + width,
        }
    }

    fn blob_radius(radius: f64, waves: &[(f64, f64); 3], c: f64, s: f64) -> f64 {
        // cos(kθ + φ) from (cos θ, sin θ) by de Moivre.
        let power = |k: usize| {
            let (mut re, mut im) = (1.0, 0.0);
            for _ in 0..k {
                (re, im) = (re * c - im * s, re * s + im * c);
            }
            (re, im)
        };
        let mut r = 1.0;
        for (&(a, phase), k) in waves.iter().zip([2, 3, 5]) {
            let (ck, sk) = power(k);
            let (sp, cp) = sin_cos(phase);
            r += a * (ck * cp - sk * sp);
        }
        radius * r
    }

    /// The outline the sea's polygon cuts out (a hole), local metres.
    pub fn outline(&self) -> Vec<[f64; 2]> {
        match self {
            Shape::Blob { radius, waves } => {
                let n = ((radius / 12.0) as usize).clamp(24, 720);
                (0..n)
                    .map(|k| {
                        let (s, c) = sin_cos(TAU * k as f64 / n as f64);
                        let r = Self::blob_radius(*radius, waves, c, s);
                        [r * c, r * s]
                    })
                    .collect()
            }
            &Shape::Spit {
                half_length,
                half_width,
                bearing_deg,
            } => {
                let (s, c) = degrees(bearing_deg);
                let (d, n) = ([c, s], [-s, c]);
                let mut out = vec![];
                let arc = 10;
                for (end, sign) in [(half_length, 1.0), (-half_length, -1.0)] {
                    for k in 0..=arc {
                        // Half a turn about the end, from one side to the other.
                        let (sa, ca) = sin_cos(PI * k as f64 / arc as f64 - PI / 2.0);
                        let (along, across) =
                            (end + sign * half_width * ca, sign * half_width * sa);
                        out.push([d[0] * along + n[0] * across, d[1] * along + n[1] * across]);
                    }
                }
                out
            }
            &Shape::Atoll { radius, width } => {
                circle(radius + width, ((radius / 10.0) as usize).clamp(48, 720))
            }
        }
    }

    /// The sea inside it (an atoll's lagoon), local metres.
    pub fn lagoon(&self) -> Option<Vec<[f64; 2]>> {
        match *self {
            Shape::Atoll { radius, width } if radius > width => Some(circle(
                radius - width,
                ((radius / 10.0) as usize).clamp(48, 720),
            )),
            _ => None,
        }
    }

    /// How far inside the DEM's land local point (x, y) lies, metres (negative outside), the coast `offset` out from the
    /// polygon's; and whether it is in an atoll's lagoon.
    fn inside(&self, x: f64, y: f64, offset: f64) -> (f64, bool) {
        match self {
            Shape::Blob { radius, waves } => {
                let rho = (x * x + y * y).sqrt();
                if rho == 0.0 {
                    return (radius + offset, false);
                }
                let r = Self::blob_radius(*radius, waves, x / rho, y / rho);
                (r + offset - rho, false)
            }
            &Shape::Spit {
                half_length,
                half_width,
                bearing_deg,
            } => {
                let (s, c) = degrees(bearing_deg);
                let t = (x * c + y * s).clamp(-half_length, half_length);
                let (dx, dy) = (x - t * c, y - t * s);
                (half_width + offset - (dx * dx + dy * dy).sqrt(), false)
            }
            &Shape::Atoll { radius, width } => {
                let rho = (x * x + y * y).sqrt();
                (width + offset - (rho - radius).abs(), rho < radius)
            }
        }
    }
}

/// An island (or reef, or spit) of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    /// Its centre: latitude, longitude (in its scene's frame).
    pub centre: [f64; 2],
    pub shape: Shape,
    /// The DEM's highest, metres.
    pub height: f64,
    /// Whether the DEM rises as a cliff (to its height within 40 m of its coast) or a dome.
    pub cliff: bool,
    /// The DEM's coast lies this far outside the polygon's (inside where negative), metres.
    pub offset: f64,
    /// Ponds: the sea's polygons inside the land, (east, north, radius) metres from the centre.
    pub ponds: Vec<[f64; 3]>,
}

impl Feature {
    pub fn new(centre: [f64; 2], shape: Shape, height: f64) -> Self {
        Self {
            centre,
            shape,
            height,
            cliff: false,
            offset: 0.0,
            ponds: vec![],
        }
    }

    fn local(&self) -> Local {
        Local::new(self.centre[0], self.centre[1])
    }

    fn rings(&self) -> (Vec<[f64; 2]>, Vec<Vec<[f64; 2]>>) {
        let local = self.local();
        let geo =
            |ring: Vec<[f64; 2]>| ring.into_iter().map(|p| local.place(p)).collect::<Vec<_>>();
        let mut water = vec![];
        if let Some(lagoon) = self.shape.lagoon() {
            water.push(geo(lagoon));
        }
        for &[x, y, r] in &self.ponds {
            water.push(geo(circle(r, ((r / 8.0) as usize).clamp(24, 360))
                .into_iter()
                .map(|[a, b]| [a + x, b + y])
                .collect()));
        }
        (geo(self.shape.outline()), water)
    }

    /// The DEM at local (x, y), none outside its reach; and whether it is water inside the land (a lagoon, a pond),
    /// which the feature alone decides.
    fn height(&self, x: f64, y: f64, glo30: bool) -> Option<(f64, bool)> {
        let (e, lagoon) = self.shape.inside(x, y, self.offset);
        if e <= 0.0 {
            if lagoon {
                return Some((-4.0, true));
            }
            let sea = -(2.0 + (-e) * 0.04);
            return (sea > FLOOR_M).then_some((sea, false));
        }
        for &[px, py, r] in &self.ponds {
            let (dx, dy) = (x - px, y - py);
            if dx * dx + dy * dy < (r - 10.0) * (r - 10.0) {
                return Some((0.3, true));
            }
        }
        let rise = if self.cliff {
            40.0
        } else {
            0.6 * self.shape.extent()
        };
        let t = (e / rise).min(1.0);
        let (s1, _) = sin_cos(x / 131.0 + 0.7);
        let (_, c1) = sin_cos(y / 97.0 - 1.3);
        let noise = 2.0 * s1 * c1 * (e / 100.0).min(1.0);
        let base = COAST_TOP_M + (self.height - COAST_TOP_M) * t + noise.max(-0.1);
        Some((if glo30 { base + 2.0 } else { base }, false))
    }
}

/// A mainland whose coast crosses a scene: east-west at latitude `at` with land north, or north-south at longitude
/// `at` with land east, wiggling.
#[derive(Clone, Debug, PartialEq)]
pub struct Mainland {
    pub east_west: bool,
    pub at: f64,
    /// Two waves of the coast: amplitude and wavelength, metres; phase, radians.
    pub waves: [(f64, f64, f64); 2],
    pub offset: f64,
    pub height: f64,
}

impl Mainland {
    fn wiggle(&self, s: f64) -> f64 {
        self.waves
            .iter()
            .map(|&(a, l, p)| a * sin_cos(TAU * s / l + p).0)
            .sum()
    }
}

/// A scene: a box of latitude and longitude with its sea, its mainland and its features.
#[derive(Clone, Debug)]
pub struct Scene {
    /// south, north, west, east (continuous).
    pub bounds: [f64; 4],
    pub mainland: Option<Mainland>,
    pub features: Vec<Feature>,
    /// The sea's polygons, made from the above ([`Scene::seal`]).
    pub ocean: Vec<Arc<GeoPolygon>>,
    /// Each feature's box with its DEM's reach: south, north, west, east.
    boxes: Vec<[f64; 4]>,
    /// The frame the mainland measures along its coast in.
    reference: Local,
}

/// The coast's sample spacing along a mainland, metres.
const MAINLAND_STEP_M: f64 = 60.0;

impl Scene {
    /// Open sea over `bounds` (south, north, west, east).
    pub fn open(bounds: [f64; 4]) -> Self {
        let reference = Local::new((bounds[0] + bounds[1]) / 2.0, (bounds[2] + bounds[3]) / 2.0);
        Self {
            bounds,
            mainland: None,
            features: vec![],
            ocean: vec![],
            boxes: vec![],
            reference,
        }
    }

    pub fn with(mut self, feature: Feature) -> Self {
        self.features.push(feature);
        self
    }

    pub fn with_mainland(mut self, mainland: Mainland) -> Self {
        self.mainland = Some(mainland);
        self
    }

    /// The mainland's across-coast distance at a place (positive on land), metres, and the coast's own place along it.
    fn mainland_across(&self, m: &Mainland, lat: f64, lon: f64) -> f64 {
        let [x, y] = self.reference.xy(lat, lon);
        if m.east_west {
            (lat - m.at) * M_PER_DEGREE - m.wiggle(x)
        } else {
            (lon - m.at) * self.reference.m_lon - m.wiggle(y)
        }
    }

    /// Makes the scene's sea polygons from its box, mainland and features.
    pub fn seal(mut self) -> Self {
        let [south, north, west, east] = self.bounds;
        let mut outer = vec![[south, west]];
        match &self.mainland {
            None => outer.extend([[south, east], [north, east], [north, west]]),
            Some(m) if m.east_west => {
                outer.push([south, east]);
                let (x0, x1) = (
                    self.reference.xy(0.0, west)[0],
                    self.reference.xy(0.0, east)[0],
                );
                let n = ((x1 - x0) / MAINLAND_STEP_M).ceil() as usize;
                for k in (0..=n).rev() {
                    let x = x0 + (x1 - x0) * k as f64 / n as f64;
                    let lon = self.reference.lon + x / self.reference.m_lon;
                    outer.push([m.at + m.wiggle(x) / M_PER_DEGREE, lon]);
                }
            }
            Some(m) => {
                let (y0, y1) = (
                    self.reference.xy(south, 0.0)[1],
                    self.reference.xy(north, 0.0)[1],
                );
                let n = ((y1 - y0) / MAINLAND_STEP_M).ceil() as usize;
                for k in 0..=n {
                    let y = y0 + (y1 - y0) * k as f64 / n as f64;
                    let lat = self.reference.lat + y / M_PER_DEGREE;
                    outer.push([lat, m.at + m.wiggle(y) / self.reference.m_lon]);
                }
                outer.push([north, west]);
            }
        }
        let mut rings = vec![outer];
        let mut water = vec![];
        self.boxes.clear();
        for f in &self.features {
            let (hole, inside) = f.rings();
            rings.push(hole);
            water.extend(inside);
            let local = f.local();
            let reach = f.shape.extent() + f.offset.max(0.0) + 30.0 / 0.04;
            let [s, w] = local.place([-reach, -reach]);
            let [n, e] = local.place([reach, reach]);
            self.boxes.push([s, n, w, e]);
        }
        self.ocean = std::iter::once(rings)
            .chain(water.into_iter().map(|ring| vec![ring]))
            .map(|rings| Arc::new(GeoPolygon::new(rings)))
            .collect();
        self
    }

    /// The longitude `lon` taken into the scene's frame, when the place lies in the scene.
    fn holds(&self, lat: f64, lon: f64) -> Option<f64> {
        let [south, north, west, east] = self.bounds;
        if lat < south || lat > north {
            return None;
        }
        let lon = lon + ((west - lon) / 360.0).ceil() * 360.0;
        (lon <= east).then_some(lon)
    }

    /// The DEM at a place in the scene (`lon` in its frame).
    fn height(&self, lat: f64, lon: f64, glo30: bool) -> f64 {
        let mut h = FLOOR_M;
        if let Some(m) = &self.mainland {
            let e = self.mainland_across(m, lat, lon) + m.offset;
            h = if e > 0.0 {
                let base = COAST_TOP_M + (m.height - COAST_TOP_M) * (e / 3000.0).min(1.0);
                if glo30 { base + 2.0 } else { base }
            } else {
                (-(2.0 + (-e) * 0.04)).max(FLOOR_M)
            };
        }
        for (f, b) in self.features.iter().zip(&self.boxes) {
            if lat < b[0] || lat > b[1] || lon < b[2] || lon > b[3] {
                continue;
            }
            let [x, y] = f.local().xy(lat, lon);
            match f.height(x, y, glo30) {
                // A pond's water or a lagoon's: the feature's own word.
                Some((v, true)) => return v,
                Some((v, false)) => h = h.max(v),
                None => {}
            }
        }
        h
    }
}

/// A splitmix64 step: the generators' only randomness.
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A uniform number in [0, 1) from a seed and a counter.
fn uniform(seed: u64, k: u64) -> f64 {
    (mix(seed ^ mix(k)) >> 11) as f64 / (1u64 << 53) as f64
}

/// Whether a place stands on a spike of the DEM, and how high: a cell's spike stands at its middle.
fn spike(lat: f64, lon: f64, salt: u64) -> f64 {
    let (ci, cj) = (
        (lat / SPIKE_CELL_DEG).floor(),
        (lon / SPIKE_CELL_DEG).floor(),
    );
    let h = mix(salt ^ mix((ci as i64 as u64) ^ mix(cj as i64 as u64)));
    if !h.is_multiple_of(SPIKE_ONE_IN) {
        return 0.0;
    }
    let local = Local::new((ci + 0.5) * SPIKE_CELL_DEG, (cj + 0.5) * SPIKE_CELL_DEG);
    let [x, y] = local.xy(lat, lon);
    if x * x + y * y < SPIKE_RADIUS_M * SPIKE_RADIUS_M {
        SPIKE_M
    } else {
        0.0
    }
}

/// The scenes.
#[derive(Clone, Debug, Default)]
pub struct World {
    pub scenes: Vec<Scene>,
}

impl World {
    pub fn new(scenes: Vec<Scene>) -> Self {
        Self { scenes }
    }

    /// The DEM at a place, any longitude: the GLO-30 stand-in past 60° N and 56° S.
    pub fn height(&self, lat: f64, lon: f64) -> f64 {
        let glo30 = !(SRTM_SOUTH..SRTM_NORTH).contains(&lat);
        let wrapped = lon - ((lon + 180.0) / 360.0).floor() * 360.0;
        for scene in &self.scenes {
            if let Some(lon) = scene.holds(lat, lon) {
                return scene.height(lat, lon, glo30) + spike(lat, wrapped, u64::from(glo30) + 1);
            }
        }
        PLATEAU_M
    }
}

impl Water for World {
    fn ocean(
        &self,
        south: f64,
        north: f64,
        west: f64,
        east: f64,
    ) -> Result<Vec<Arc<GeoPolygon>>, BakeError> {
        let mut out = vec![];
        for scene in &self.scenes {
            let [s, n, w, e] = scene.bounds;
            if n < south || s > north {
                continue;
            }
            let lo = ((west - e) / 360.0).ceil() as i64;
            let hi = ((east - w) / 360.0).floor() as i64;
            if lo <= hi {
                out.extend(scene.ocean.iter().cloned());
            }
        }
        Ok(out)
    }
}

/// The synthetic sources: the world, and the surveyed squares baked from its DEM as they are asked for.
pub struct SyntheticSources {
    pub world: Arc<World>,
    squares: Mutex<BTreeMap<SquareKey, Arc<Square>>>,
}

impl SyntheticSources {
    pub fn new(world: Arc<World>) -> Self {
        Self {
            world,
            squares: Mutex::new(BTreeMap::new()),
        }
    }

    /// The surveyed squares baked so far.
    pub fn baked(&self) -> usize {
        self.squares.lock().unwrap().len()
    }
}

impl Surveyed for SyntheticSources {
    fn square(&self, key: SquareKey) -> Result<Arc<Square>, BakeError> {
        if let Some(s) = self.squares.lock().unwrap().get(&key) {
            return Ok(s.clone());
        }
        let world = &self.world;
        let dem = if (key.lat as f64) < SRTM_NORTH && (key.lat + 1) as f64 > SRTM_SOUTH {
            Dem::Terrarium
        } else {
            Dem::Glo30
        };
        let made = Arc::new(square::bake_sampled(
            key,
            &|lat, lon| world.height(lat, lon),
            SquareSource {
                dem,
                zoom: None,
                fallback: false,
                attribution: "synthetic".into(),
            },
        ));
        Ok(self
            .squares
            .lock()
            .unwrap()
            .entry(key)
            .or_insert(made)
            .clone())
    }
}

impl Water for SyntheticSources {
    fn ocean(
        &self,
        south: f64,
        north: f64,
        west: f64,
        east: f64,
    ) -> Result<Vec<Arc<GeoPolygon>>, BakeError> {
        self.world.ocean(south, north, west, east)
    }
}

/// The catalogue [`Scene::across`] lays along an edge, in order: each a shape, its DEM's height, cliff or dome, the
/// DEM's coast offset, and ponds.
/// The second bake's sea floor over a synthetic world: the world's DEM where it stands above the sea (the bathymetry's
/// own coast, as a real grid's runs a pixel off the polygons'), else a floor from 3 to 400 m deep that rolls with the
/// place (banks a ship grounds on, deeps past the shelf's 160 m). `open` are the squares the release calls open sea.
pub struct SyntheticSeabed {
    pub world: Arc<World>,
    pub open: std::collections::BTreeSet<SquareKey>,
}

impl SyntheticSeabed {
    /// The floor at a place, any longitude.
    pub fn at(&self, lat: f64, lon: f64) -> f64 {
        let h = self.world.height(lat, lon);
        if h > 0.5 {
            return h;
        }
        let lon = lon - ((lon + 180.0) / 360.0).floor() * 360.0;
        let (a, _) = sin_cos(lat * 0.9);
        let (_, b) = sin_cos(lon * 0.7);
        let (c, _) = sin_cos((lat + lon) * 3.1);
        -(3.0 + 397.0 * (0.5 + 0.2 * a * b + 0.45 * c).clamp(0.0, 1.0))
    }
}

impl super::Seabed for SyntheticSeabed {
    fn floor(
        &self,
        region: &super::lattice::Region,
    ) -> Result<super::lattice::Plane<f64>, BakeError> {
        let per = f64::from(naval_land_format::lattice::PER_DEGREE);
        Ok(super::lattice::Plane::from_fn(*region, |r, c| {
            let (i, j) = (region.i(r), super::lattice::wrap_j(region.j(c)));
            self.at(f64::from(i) / per, f64::from(j) / per)
        }))
    }

    fn open_sea(&self, key: SquareKey) -> bool {
        self.open.contains(&key) || naval_land_format::table::polar(key)
    }
}

fn catalogue(k: usize, seed: u64) -> (Shape, f64, bool, f64, Vec<[f64; 3]>) {
    let wave = |j: u64, a: f64| (a * (0.5 + uniform(seed, j)), TAU * uniform(seed, j + 7));
    let waves = |a: f64| [wave(1, a), wave(2, a * 0.6), wave(3, a * 0.3)];
    match k % 10 {
        // A big island, cliffs, a pond under the cap (filled) and one over it (kept), its DEM coast 50 m out.
        0 => (
            Shape::Blob {
                radius: 2500.0,
                waves: waves(0.12),
            },
            280.0,
            true,
            50.0,
            vec![[-700.0, 500.0, 120.0], [600.0, -350.0, 650.0]],
        ),
        // A speck.
        1 => (
            Shape::Blob {
                radius: 28.0,
                waves: [(0.0, 0.0); 3],
            },
            8.0,
            true,
            0.0,
            vec![],
        ),
        // A spit rounding sinks, under the cap: kept whole.
        2 => (
            Shape::Spit {
                half_length: 260.0,
                half_width: 28.0,
                bearing_deg: 360.0 * uniform(seed, 11),
            },
            4.0,
            false,
            0.0,
            vec![],
        ),
        // An atoll whose lagoon is over the cap: the lagoon stays sea.
        3 => (
            Shape::Atoll {
                radius: 3000.0,
                width: 260.0,
            },
            3.0,
            false,
            20.0,
            vec![],
        ),
        // A low island, its DEM coast 90 m inside the polygon's (the slide).
        4 => (
            Shape::Blob {
                radius: 450.0,
                waves: waves(0.2),
            },
            7.0,
            false,
            -90.0,
            vec![],
        ),
        // A small atoll: its lagoon under the cap fills.
        5 => (
            Shape::Atoll {
                radius: 280.0,
                width: 110.0,
            },
            3.0,
            false,
            0.0,
            vec![],
        ),
        // A dome whose DEM runs 160 m past the polygon's coast (the cut).
        6 => (
            Shape::Blob {
                radius: 900.0,
                waves: waves(0.25),
            },
            120.0,
            false,
            160.0,
            vec![],
        ),
        // A small island, three samples or so: not a speck.
        7 => (
            Shape::Blob {
                radius: 70.0,
                waves: [(0.0, 0.0); 3],
            },
            15.0,
            true,
            0.0,
            vec![],
        ),
        // A spit over the cap: rounding sinks it and it is not kept.
        8 => (
            Shape::Spit {
                half_length: 1300.0,
                half_width: 30.0,
                bearing_deg: 360.0 * uniform(seed, 13),
            },
            3.0,
            false,
            0.0,
            vec![],
        ),
        // An irregular island.
        _ => (
            Shape::Blob {
                radius: 1200.0,
                waves: waves(0.35),
            },
            60.0,
            false,
            -30.0,
            vec![],
        ),
    }
}

impl Scene {
    /// A scene across the edge squares `a` and `b` share (`b` east or north of `a`), with a mainland's coast crossing
    /// the edge and features laid along it, the edge cutting each at a place the seed chooses. Its box holds both
    /// squares and their halos.
    pub fn across(a: SquareKey, b: SquareKey, seed: u64) -> Self {
        let east = b == a.east();
        assert!(
            east || Some(b) == a.north(),
            "{b} is not east or north of {a}"
        );
        let halo = Halo::of(a).columns().max(Halo::of(b).columns()) as f64 / 1600.0;
        let (lat0, lon0) = (f64::from(a.lat), f64::from(a.lon));
        let bounds = [
            (lat0 - 0.12).max(-89.4),
            (lat0 + if east { 1.0 } else { 2.0 } + 0.12).min(89.4),
            lon0 - halo - 0.05,
            lon0 + if east { 2.0 } else { 1.0 } + halo + 0.05,
        ];
        let mut scene = Scene::open(bounds);
        // The mainland: across the edge in the far part of it.
        let wiggle =
            |j: u64, a: f64, l: f64| (a * (0.5 + uniform(seed, j)), l, TAU * uniform(seed, j + 3));
        scene.mainland = Some(if east {
            Mainland {
                east_west: true,
                at: lat0 + 0.82,
                waves: [wiggle(21, 900.0, 7000.0), wiggle(22, 300.0, 1900.0)],
                offset: 40.0,
                height: 300.0,
            }
        } else {
            let length = M_PER_DEGREE * degrees(lat0 + 1.0).1;
            Mainland {
                east_west: false,
                at: lon0 + 0.85,
                waves: [
                    wiggle(21, (0.03 * length).min(900.0), 7000.0),
                    wiggle(22, 300.0, 1900.0),
                ],
                offset: 40.0,
                height: 300.0,
            }
        });
        // Features along the edge, from a little past its start to short of the mainland.
        let edge = if east {
            Local::new(lat0, lon0 + 1.0)
        } else {
            Local::new(lat0 + 1.0, lon0)
        };
        let (start, end) = if east {
            (0.03 * M_PER_DEGREE, 0.74 * M_PER_DEGREE)
        } else {
            (0.03 * edge.m_lon, 0.78 * edge.m_lon - 1200.0)
        };
        let mut along = start;
        let mut k = 0usize;
        let mut misses = 0;
        while misses < 10 {
            let (shape, height, cliff, offset, ponds) = catalogue(k, mix(seed ^ k as u64));
            let extent = shape.extent() + offset.max(0.0);
            let centre_along = along + extent + 150.0;
            k += 1;
            if centre_along + extent > end {
                misses += 1;
                continue;
            }
            // The edge cuts the feature at a place the seed chooses; a third of them on their centres.
            let across = if k.is_multiple_of(3) {
                0.0
            } else {
                (uniform(seed, 100 + k as u64) - 0.5) * 1.4 * shape.extent()
            };
            let xy = if east {
                [across, centre_along]
            } else {
                [centre_along, across]
            };
            scene.features.push(Feature {
                centre: edge.place(xy),
                shape,
                height,
                cliff,
                offset,
                ponds,
            });
            along = centre_along + extent + 150.0;
        }
        scene.seal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finish::coverage::{HALF, ocean_cover};
    use crate::finish::lattice::Region;

    #[test]
    fn a_scene_is_sea_where_its_polygons_say_and_its_dem_mostly_agrees() {
        let (a, b) = (
            SquareKey { lat: 10, lon: 20 },
            SquareKey { lat: 10, lon: 21 },
        );
        let scene = Scene::across(a, b, 3);
        assert!(scene.features.len() >= 10, "{}", scene.features.len());
        let world = World::new(vec![scene]);
        // A strip of the edge: the coverage and the DEM agree away from the coasts.
        let region = Region {
            top: 10 * 1600 + 1200,
            left: 21 * 1600 - 200,
            rows: 1100,
            columns: 400,
        };
        let cover = ocean_cover(&region, &world).unwrap();
        let (mut land, mut agree) = (0, 0);
        for r in 0..region.rows {
            for c in 0..region.columns {
                let p = region.point(r, c);
                let sea = cover.at(r, c) >= HALF;
                land += usize::from(!sea);
                agree += usize::from(sea == (world.height(p.lat(), p.lon()) <= 0.5));
            }
        }
        assert!(land > 10_000, "{land}");
        assert!(
            agree as f64 > 0.97 * region.len() as f64,
            "{agree} of {}",
            region.len()
        );
        // Outside every scene the world is land.
        assert_eq!(world.height(-40.0, 100.0), PLATEAU_M);
    }

    #[test]
    fn a_shapes_outline_is_where_its_dem_coast_is_with_no_offset() {
        for shape in [
            Shape::Blob {
                radius: 500.0,
                waves: [(0.2, 1.0), (0.1, 2.0), (0.05, 0.3)],
            },
            Shape::Spit {
                half_length: 300.0,
                half_width: 30.0,
                bearing_deg: 33.0,
            },
            Shape::Atoll {
                radius: 400.0,
                width: 80.0,
            },
        ] {
            for p in shape.outline() {
                let (e, _) = shape.inside(p[0], p[1], 0.0);
                assert!(e.abs() < 1.0, "{shape:?} {p:?} {e}");
            }
        }
    }
}
