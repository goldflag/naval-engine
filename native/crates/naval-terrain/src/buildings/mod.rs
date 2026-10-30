//! Buildings over a theatre's land, as flight simulators draw them (native/docs/terrain/buildings.md): real
//! footprints where the host has them (Microsoft's Global ML Building Footprints and OpenStreetMap's outlines, fetched
//! by `naval-client`), generic houses and blocks scattered over built-up land where it has none, each extruded from the
//! ground under it with a flat, gabled, hipped or sawtooth roof and a procedural facade, typed by the map about it
//! (churches, warehouses, factories, oil tanks, lighthouses, quays' cranes, piers), in chunks of a fixed grid on the
//! theatre's metres.
//!
//! The crate stays portable: it reads no file and opens no connection. The host hands it footprints in chunk-local
//! metres, the map's tags about them ([`MapTags`]) and a way to read the ground; [`build_chunk`] makes a chunk's mesh
//! (on any thread), and [`BuildingsLayer`] draws the chunks it is given:
//!
//! ```ignore
//! let mesh = build_chunk(&ChunkInput { key, footprints: &real, infill: &generic, tags: &tags, ground: &|p| h(p), region, period, thinning });
//! layer.queue_chunk(key, Arc::new(mesh));     // uploaded from the layer's own prepare
//! layer.set_offset(offset);                   // world x, z = theatre x, z + offset
//! ```
//!
//! - **Footprints** ([`Footprint`]): a ring of (x, z) metres from the chunk's corner (x east, z south), and the source's
//!   height and colour when it has them.
//! - **Tags** ([`tags`]): the land use about a footprint, the point of interest on it, the sea and the piers, OSM's
//!   outlines (their heights where Microsoft's are missing or capped), the landmarks the host knows.
//! - **Plan** ([`plan_building`]): each footprint's kind, storeys, roof, colours and what stands with it (a church's
//!   tower, a mosque's dome and minarets, a factory's chimneys), from its setting, area, shape, height and the share of
//!   the ground built over about it, in the region's palettes ([`Region::at`]).
//! - **Mesh** ([`mesh`], [`parts`]): walls from just below the lowest ground under the footprint to the eaves, the roof
//!   and the extras, each vertex 24 bytes ([`BuildingVertex`]); a [`Lod::Box`] chunk draws each building as a box under
//!   a flat top, its tall extras simplified and its cranes and spires' detail dropped.
//! - **Infill** ([`infill`]): lots on a street grid over built-up land the footprints leave empty, deterministic in the
//!   theatre's metres.
//! - **The far level** ([`far`], [`Lod::Far`]): past the boxes' range, a town as coarse masses on the infill's street
//!   grid over the land cover's built-up land, no footprints read.

pub mod clutter;
pub mod eaves;
pub mod facades;
pub mod far;
pub mod geometry;
pub mod grounds;
pub mod history;
pub mod infill;
pub mod layer;
pub mod mesh;
pub mod palette;
pub mod parcels;
pub mod parts;
pub mod period;
pub mod region;
pub mod street;
pub mod tags;
pub mod thinning;
pub mod towns;

pub use layer::{BuildingSettings, BuildingsLayer};
pub use mesh::{BuildingVertex, ChunkMesh, build_chunk};
pub use palette::{KAWARA, THATCH};
use palette::{colours, detail_share, roof_rules};
pub use period::Period;
pub use region::{Region, mexican_border};
pub use tags::{Landmark, LandmarkKind, MapTags, PoiKind, Religion, Setting, Usage};

/// A chunk's side, metres of the theatre: chunk (i, j) covers theatre x from i × [`CHUNK_M`], z likewise.
pub const CHUNK_M: f64 = 1000.0;

/// The chunk holding theatre point `p`.
pub fn chunk_of(p: [f64; 2]) -> [i32; 2] {
    [
        (p[0] / CHUNK_M).floor() as i32,
        (p[1] / CHUNK_M).floor() as i32,
    ]
}

/// The theatre point of chunk `index`'s corner (its least x and z).
pub fn chunk_corner(index: [i32; 2]) -> [f64; 2] {
    [f64::from(index[0]) * CHUNK_M, f64::from(index[1]) * CHUNK_M]
}

/// How much of a chunk is drawn: every building with its roof and facade, each as a box under a flat top, or (past the
/// boxes' range) its town as coarse masses made from the land cover alone ([`far`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lod {
    Full,
    Box,
    Far,
}

/// A chunk at one level of detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkKey {
    pub index: [i32; 2],
    pub lod: Lod,
}

/// A building's outline as the source gives it: a ring of chunk-local (x, z) metres, its height (m, to its top) and its
/// walls' colour (sRGB) when the source has them, its kind when the source knows it (the infill's lots), and for one of
/// OpenStreetMap's building parts where its walls start and the building it belongs to ([`Part`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Footprint {
    pub ring: Vec<[f32; 2]>,
    pub height: Option<f32>,
    pub kind: Option<Kind>,
    pub colour: Option<[u8; 3]>,
    pub part: Option<Part>,
}

/// One of OpenStreetMap's `building:part`s standing as a footprint of its own (`tags::merge_outlines`): a tower mapped
/// as parts stacked one on another stands as they are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Part {
    /// Where its walls start over its building's ground (m, OSM's `min_height`; 0 for a part on the ground): above it
    /// nothing reaches down to the ground, nor sinks below its base ([`mesh::SINK_M`]).
    pub base: f32,
    /// The middle of the building it is part of (chunk-local x, z): its plan's hashes and its ground are read there, so
    /// the parts of one tower take one colour, one storey and windows that run on from part to part.
    pub middle: [f32; 2],
}

impl Footprint {
    /// Whether it starts above the ground: one of OSM's parts with a base ([`Part::base`]).
    pub fn raised(&self) -> bool {
        self.part.is_some_and(|p| p.base > 0.0)
    }
}

/// What a building is, for its height, roof, colours and windows (`buildings.wgsl`'s `KIND_*`, four bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A shed, garage or hut: one low storey.
    Shed = 0,
    /// A detached house.
    House = 1,
    /// A terrace or row of houses, or a small block.
    Terrace = 2,
    /// A block of flats.
    Block = 3,
    /// A tower (where a source's height says so, or now and then in a dense core).
    Tower = 4,
    /// A factory: a large low shed with a high ribbon of windows, a sawtooth roof now and then, chimneys.
    Industrial = 5,
    /// A church, mosque, synagogue or temple: tall windows, its tower, dome or minarets beside it.
    Church = 6,
    /// Offices on commercial land: ribbons of glass or a curtain wall.
    Office = 7,
    /// A warehouse or quay shed: big doors, a low pitched roof.
    Warehouse = 8,
    /// An oil or water tank: a cylinder under a shallow cone.
    Tank = 9,
    /// A lighthouse: its tower, gallery and lantern ([`Extra::Light`]).
    Lighthouse = 10,
    /// A free-standing stack ([`Extra::Chimney`]).
    Chimney = 11,
    /// A crane on a quay ([`Extra::Crane`]).
    Crane = 12,
    /// A pier's deck.
    Deck = 13,
}

/// A building's roof.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Roof {
    Flat,
    /// A ridge along the ring's first side (the footprint's longer, or a terraced house's width, along its street),
    /// its rise (m) above the eaves.
    Gabled {
        rise: f32,
    },
    /// Four slopes, its rise (m).
    Hipped {
        rise: f32,
    },
    /// A factory's north lights: teeth `tooth` m apart across the long side, each rising `rise` to a glazed face.
    Sawtooth {
        rise: f32,
        tooth: f32,
    },
    /// A tank's shallow cone, its rise (m).
    Cone {
        rise: f32,
    },
}

impl Roof {
    /// The roof's rise above the eaves (m).
    pub fn rise(&self) -> f32 {
        match *self {
            Roof::Flat => 0.0,
            Roof::Gabled { rise }
            | Roof::Hipped { rise }
            | Roof::Sawtooth { rise, .. }
            | Roof::Cone { rise } => rise,
        }
    }
}

/// A church tower's top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    /// A spire `height` m over the tower, roofed in palette entry `colour`.
    Spire {
        height: f32,
        colour: u8,
    },
    /// A low pyramid (a campanile's), `height` m.
    Pyramid {
        height: f32,
        colour: u8,
    },
    Flat,
}

/// What stands with a building besides its walls and roof. Heights are metres over the building's mean ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Extra {
    /// A square tower from the ground to `top`, `half` its half side, turned to `axis`, under its cap.
    Tower {
        centre: [f32; 2],
        axis: [f32; 2],
        half: f32,
        top: f32,
        cap: Cap,
    },
    /// A dome of `radius` on a short drum whose foot stands `base` over the ground, roofed in entry `colour`.
    Dome {
        centre: [f32; 2],
        radius: f32,
        base: f32,
        colour: u8,
    },
    /// A minaret `radius` m to `top`, its gallery a little below, a pencil cap in entry `colour`.
    Minaret {
        centre: [f32; 2],
        radius: f32,
        top: f32,
        colour: u8,
    },
    /// A stack `radius` m at its foot to `top`, red and white bands at its head when `banded`.
    Chimney {
        centre: [f32; 2],
        radius: f32,
        top: f32,
        banded: bool,
    },
    /// A lighthouse's tower (`radius` at its foot) to its gallery at `top`, `bands` red (or black) bands up it, its
    /// lantern and cap (red or black) over the gallery.
    Light {
        centre: [f32; 2],
        radius: f32,
        top: f32,
        bands: u8,
        red: bool,
    },
    /// A crane on a quay at `at`, its jib or boom out over the water toward `out`: a container gantry or a jib crane,
    /// painted entry `colour` (the colour cube's).
    Crane {
        at: [f32; 2],
        out: [f32; 2],
        gantry: bool,
        colour: u8,
    },
}

/// A footprint planned for its mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Building {
    /// Chunk-local ring, counter-clockwise in (x, z); four corners under a pitched or sawtooth roof.
    pub ring: Vec<[f32; 2]>,
    pub kind: Kind,
    pub roof: Roof,
    /// The ground under it (m): least, mean and most over its corners and middle (a pier's deck for one on a pier).
    pub ground: [f32; 3],
    /// The walls' height above the mean ground (m): to the eaves under a pitched roof.
    pub height: f32,
    /// Where its walls start over the mean ground (m): an OSM part's base ([`Part::base`]); 0 for one standing on the
    /// ground, whose walls reach [`mesh::SINK_M`] below its lowest ground.
    pub base: f32,
    /// Window rows, and a storey's height (m).
    pub storeys: u8,
    pub storey_m: f32,
    /// Palette entries: the walls' and the roof's (`buildings.wgsl`'s `wall_colour` and `roof_colour`: 32 authored,
    /// then a 6 × 6 × 6 colour cube, [`cube_entry`]).
    pub wall: u8,
    pub roof_colour: u8,
    /// A per-building hash, 0-255: tint and window variety.
    pub seed: u8,
    /// Shutters on a house or terrace, balconies on a block or tower, a curtain wall on an office.
    pub detail: bool,
    /// A window bay's width (m) along its walls.
    pub bay: f32,
    /// What stands with it: a church's tower, a dome, minarets, a factory's chimneys, a lighthouse's tower.
    pub extras: Vec<Extra>,
    /// The age it is drawn in ([`Period`]).
    pub period: Period,
}

/// The integer hash the buildings use (the web noise's), in 0-1.
pub fn hash01(x: i64, y: i64, seed: i64) -> f32 {
    let (x, y, seed) = (x as u32, y as u32, seed as u32);
    let mut h =
        x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h = (h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35);
    (f64::from(h ^ (h >> 16)) / 4_294_967_296.0) as f32
}

/// The most a source's height may be and still be taken (m): the tallest towers (the Burj Khalifa's 828 m, Taipei 101's
/// 508 m); above it, or below [`MIN_DATA_HEIGHT_M`], the height is estimated instead.
pub const MAX_DATA_HEIGHT_M: f32 = 850.0;
/// The least height taken from a source (m): Microsoft's model returns 0-2 m for many sheds and for footprints it could
/// not measure.
pub const MIN_DATA_HEIGHT_M: f32 = 2.5;
/// The most storeys an estimate gives a block (no data); a rare tower in a dense core and Taiwan's towers aside.
pub const MAX_ESTIMATED_STOREYS: u8 = 8;
/// The first palette entry of the colour cube (`buildings.wgsl`): entries below it are the authored palettes.
pub const CUBE_FIRST: u8 = 32;

/// The colour cube's entry nearest an sRGB colour: six levels a channel (0, 51, ... 255).
pub fn cube_entry(rgb: [u8; 3]) -> u8 {
    let level = |c: u8| ((f32::from(c) / 51.0).round() as u8).min(5);
    CUBE_FIRST + level(rgb[0]) * 36 + level(rgb[1]) * 6 + level(rgb[2])
}

/// Picks one of `options` by `t` in 0-1.
pub(crate) fn pick(options: &[u8], t: f32) -> u8 {
    options[((t * options.len() as f32) as usize).min(options.len() - 1)]
}

/// Picks one of `options` by a byte hash.
pub(crate) fn pick_entry(options: &[u8], seed: u8) -> u8 {
    options[usize::from(seed) * options.len() / 256]
}

/// Plans `footprint` (chunk-local, any orientation) of chunk `chunk` for its mesh: its kind from the map's `setting`
/// (a church on a place of worship, warehouses and factories on industrial land, offices on commercial land, an oil
/// tank where a round footprint stands on industrial land or by the sea, a lighthouse), else from the source's height
/// when sane, else from its area, its shape and `density` (the share of the ground about it built over, 0-1); a pitched
/// roof on a small near-rectangular one; the ground under it from `ground` (chunk-local (x, z) to metres above the sea).
/// None for a footprint too small, degenerate, or standing in the sea off any pier.
pub fn plan_building(
    footprint: &Footprint,
    chunk: [i32; 2],
    density: f32,
    region: Region,
    period: Period,
    setting: &Setting,
    ground: &dyn Fn([f32; 2]) -> f32,
) -> Option<Building> {
    let ring = geometry::simplify(&footprint.ring, 0.4)?;
    let area = geometry::area(&ring);
    // A raised part may be a tower's mast or spire.
    if area < if footprint.raised() { 1.0 } else { 6.0 } {
        return None;
    }
    let centre = geometry::centroid(&ring);
    // A whole building, or one of OSM's parts ([`Part`]): a part's hashes and mean ground are its building's.
    let whole = footprint.part.is_none();
    let anchor = footprint.part.map_or(centre, |p| p.middle);
    // The building's own hashes, by where it stands in the theatre (decimetres): the same whichever window draws it.
    // A part's building's middle may lie in the chunk beside it (`tags::merge_outlines`): rounded, so both agree.
    let dm = |v: f32| {
        if whole {
            (v * 10.0) as i64
        } else {
            (v * 10.0).round() as i64
        }
    };
    let hx = i64::from(chunk[0]) * 10_000 + dm(anchor[0]);
    let hz = i64::from(chunk[1]) * 10_000 + dm(anchor[1]);
    let h = |k: i64| hash01(hx, hz, k);
    let rect = geometry::min_area_rect(&ring)?;
    let rectangular = area / rect.area().max(1e-3);
    let short = 2.0 * rect.half[1];
    let long = 2.0 * rect.half[0];

    // The ground under it: its corners and its middle (a pier's deck under one on a pier).
    let (lo, mean, hi) = if let Some(deck) = setting.pier {
        (deck, deck, deck)
    } else {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        let mut sum = 0.0;
        let mut count = 0.0;
        for p in ring.iter().chain(std::iter::once(&centre)) {
            let g = ground(*p);
            lo = lo.min(g);
            hi = hi.max(g);
            sum += g;
            count += 1.0;
        }
        // In the sea by the window's heights (a coast a sample off): left out.
        if ground(centre) < -1.0 || hi < 0.2 {
            return None;
        }
        // On a cliff: left out (the heights' 40-140 m samples put coastal buildings on the shelf's face).
        if hi - lo > 6.0 + 0.25 * long {
            return None;
        }
        // A part's storeys counted from its building's middle, so stacked parts meet and their windows run on.
        let mean = if whole { sum / count } else { ground(anchor) };
        (lo, mean, hi)
    };

    let data = footprint
        .height
        .filter(|h| (MIN_DATA_HEIGHT_M..=MAX_DATA_HEIGHT_M).contains(h));
    let round = ring.len() >= 6 && geometry::roundness(&ring) >= 0.86;
    let diameter = 2.0 * (area / std::f32::consts::PI).sqrt();
    let industrial = setting.usage == Usage::Industrial;
    let worship = match setting.poi {
        Some(PoiKind::Worship(r)) => Some(r),
        _ => None,
    };
    let jitter = h(1);

    let (mut kind, mut walls_m) = match footprint.kind {
        // The infill's lot: on industrial land a shed or warehouse.
        Some(kind) => {
            let height = footprint.height.unwrap_or(6.0);
            if industrial && kind != Kind::Shed {
                (Kind::Warehouse, 6.0 + 4.0 * jitter)
            } else {
                (kind, height)
            }
        }
        // A part stands as mapped: none of a lighthouse's, a church's, a stack's or a tank's own parts made about it.
        None if whole && setting.poi == Some(PoiKind::Lighthouse) && area < 400.0 => {
            (Kind::Lighthouse, data.unwrap_or(14.0 + 10.0 * jitter))
        }
        None if whole && worship.is_some() && (40.0..8000.0).contains(&area) => (Kind::Church, 0.0),
        // A measured stack before any tank: tall for its width.
        None if whole
            && round
            && diameter <= 25.0
            && data.is_some_and(|t| t >= 20.0 && t >= 2.5 * diameter) =>
        {
            (Kind::Chimney, data.unwrap_or(30.0))
        }
        None if whole
            && round
            && (9.0..=110.0).contains(&diameter)
            && data.is_none_or(|h| h < 30.0)
            && (industrial
                || (setting.usage == Usage::Unknown && diameter >= 12.0)
                || (setting.near_sea
                    && matches!(setting.usage, Usage::Unknown | Usage::Commercial))) =>
        {
            let d = diameter;
            (
                Kind::Tank,
                (4.0 + 0.3 * d).clamp(6.0, 22.0) * (0.85 + 0.3 * h(19)),
            )
        }
        None => match data {
            Some(height) => (measured_kind(height, area, setting.usage, h(12)), height),
            None => {
                let (kind, storeys) = estimate(
                    area,
                    long / short.max(0.5),
                    density,
                    jitter,
                    region,
                    setting.usage,
                );
                let pad = if matches!(kind, Kind::Industrial | Kind::Warehouse) {
                    0.0
                } else {
                    0.4
                };
                (kind, f32::from(storeys) * storey_height(kind) + pad)
            }
        },
    };
    // A rare tower in a dense core.
    if whole
        && footprint.kind.is_none()
        && data.is_none()
        && kind == Kind::Block
        && density > 0.5
        && (500.0..2500.0).contains(&area)
        && long / short.max(0.5) < 2.2
        && h(9) < 0.03
        && !period.is_past()
        && !matches!(
            region,
            Region::Tropical | Region::Arid | Region::Micronesia | Region::Atoll
        )
    {
        kind = Kind::Tower;
        walls_m = (10.0 + (h(13) * 8.99).floor()) * storey_height(Kind::Tower) + 0.4;
    }
    // The period's estimated buildings (and the infill's lots) lower: no more than its region's storeys
    // (`thinning::period_storeys`).
    if period.is_past()
        && (footprint.kind.is_some() || data.is_none())
        && matches!(
            kind,
            Kind::House | Kind::Terrace | Kind::Block | Kind::Tower | Kind::Office
        )
    {
        if kind == Kind::Tower {
            kind = Kind::Block;
        }
        let most = f32::from(thinning::period_storeys(region)) * storey_height(kind) + 0.4;
        walls_m = walls_m.min(most);
    }
    // A measured one taller than its place allowed (`Setting::cap`): the kind its height gave, a tower a block, at the
    // cap. Churches, chimneys, tanks and lighthouses keep their heights.
    if period.is_past()
        && let Some(cap) = setting.cap
        && footprint.kind.is_none()
        && data.is_some_and(|h| h > cap)
        && matches!(
            kind,
            Kind::Shed
                | Kind::House
                | Kind::Terrace
                | Kind::Block
                | Kind::Tower
                | Kind::Office
                | Kind::Industrial
                | Kind::Warehouse
        )
    {
        if kind == Kind::Tower {
            kind = Kind::Block;
        }
        walls_m = walls_m.min(cap);
    }
    // A part from its base up: none at all where it would start at its top (over the period's cap).
    let base = footprint.part.map_or(0.0, |p| p.base);
    if base > 0.0 && walls_m < base + 1.0 {
        return None;
    }
    let storey_m = storey_height(kind);
    // A part too narrow for a room (a mast, a spire): plain walls, its window rows ending below it.
    let storeys = if !whole && short < 4.0 {
        1
    } else {
        storeys_of(walls_m, storey_m)
    };
    let mut extras = Vec::new();
    let mut detail = h(17) < detail_share(region, kind);
    // No curtain wall in the period: an office of the time is masonry.
    if period.is_past() && kind == Kind::Office {
        detail = false;
    }
    let bay = bay_m(kind) * (0.85 + 0.35 * h(18));
    let seed = (h(7) * 255.0) as u8;

    let (ring, roof, height, storeys, storey_m, pitched) = match kind {
        Kind::Church => {
            let religion = worship.unwrap_or(Religion::Christian);
            church(
                &ring,
                &rect,
                area,
                rectangular,
                data,
                religion,
                region,
                &h,
                &mut extras,
            )
        }
        Kind::Tank => {
            let ring = geometry::regular(centre, diameter * 0.5, 20, h(20) * 0.3);
            let roof = if h(21) < 0.7 {
                Roof::Cone {
                    rise: 0.08 * diameter,
                }
            } else {
                Roof::Flat
            };
            (ring, roof, walls_m, 1, walls_m.max(1.0), false)
        }
        Kind::Lighthouse => {
            if area > 70.0 {
                // A tower rising from its keeper's house: the house under a pitched roof, the tower through it.
                kind = Kind::House;
                let rise = rect.half[1] * 30f32.to_radians().tan();
                extras.push(Extra::Light {
                    centre,
                    radius: 1.8,
                    top: walls_m.max(4.0 + rise + 4.0),
                    bands: 0,
                    red: false,
                });
                (
                    rect.corners().to_vec(),
                    Roof::Gabled { rise },
                    3.4,
                    1,
                    2.8,
                    true,
                )
            } else {
                extras.push(Extra::Light {
                    centre,
                    radius: (diameter * 0.5).clamp(1.6, 4.5),
                    top: walls_m,
                    bands: if h(22) < 0.3 { 3 } else { 0 },
                    red: h(23) < 0.6,
                });
                (ring, Roof::Flat, walls_m, 1, walls_m, false)
            }
        }
        Kind::Chimney => {
            extras.push(Extra::Chimney {
                centre,
                radius: (diameter * 0.5).clamp(1.0, 9.0),
                top: walls_m,
                banded: walls_m > 45.0,
            });
            (ring, Roof::Flat, walls_m, 1, walls_m, false)
        }
        Kind::Industrial | Kind::Warehouse => {
            let rect_like = rectangular > 0.8;
            // A part under a flat top, as any part's: no roof over its mapped top, nor into the part standing on it.
            let roof = if !whole {
                Roof::Flat
            } else if kind == Kind::Industrial
                && rect_like
                && short >= 14.0
                && long >= 20.0
                && h(14) < 0.6
            {
                let tooth = 7.0 + 3.0 * h(24);
                Roof::Sawtooth {
                    rise: tooth * 0.45,
                    tooth,
                }
            } else if rect_like && short <= 60.0 && (kind == Kind::Warehouse || h(14) < 0.8) {
                let pitch = 8.0 + 6.0 * h(3);
                Roof::Gabled {
                    rise: rect.half[1] * pitch.to_radians().tan(),
                }
            } else {
                Roof::Flat
            };
            // A factory's chimneys (on the map's industrial land only): one or two through its roof near an end.
            if whole && kind == Kind::Industrial && industrial && area >= 900.0 && h(15) < 0.35 {
                let n = if h(16) < 0.35 { 2 } else { 1 };
                let top = walls_m + 15.0 + 30.0 * h(25);
                let side = if h(26) < 0.5 { -1.0 } else { 1.0 };
                let normal = [-rect.axis[1], rect.axis[0]];
                for k in 0..n {
                    let along = side * (rect.half[0] - 4.0).max(0.0);
                    let across = (k as f32 - (n as f32 - 1.0) * 0.5) * 7.0;
                    extras.push(Extra::Chimney {
                        centre: [
                            rect.centre[0] + rect.axis[0] * along + normal[0] * across,
                            rect.centre[1] + rect.axis[1] * along + normal[1] * across,
                        ],
                        radius: 1.0 + 1.2 * h(27 + k),
                        top,
                        banded: top > 40.0,
                    });
                }
            }
            let corners = !matches!(roof, Roof::Flat);
            let ring = if corners {
                rect.corners().to_vec()
            } else {
                ring
            };
            (ring, roof, walls_m, storeys, storey_m, corners)
        }
        _ => {
            // A pitched roof on a small, near-rectangular house, shed or terrace; a hipped tile roof on some low blocks;
            // each region's share flat, pitch and share hipped (`palette::roof_rules`).
            let rules = roof_rules(region, kind, period);
            let small = matches!(kind, Kind::House | Kind::Shed | Kind::Terrace)
                && rectangular > 0.8
                && short <= 16.0
                && walls_m < 13.0
                && h(2) >= rules.flat;
            let block = kind == Kind::Block
                && rectangular > 0.85
                && short <= 22.0
                && storeys <= 6
                && h(28) < rules.block;
            // A part under a flat top: the parts above stand on it.
            if whole && (small || block) {
                // A house of a terrace (one narrow footprint of a row: no longer than 18 m, a quarter longer than
                // wide) has its ridge across its width, along the street, so the row's roofs run on as one.
                let across = kind == Kind::Terrace
                    && rect.half[0] <= 9.0
                    && rect.half[0] >= 1.25 * rect.half[1];
                // A row's ridges nearly level: a quarter of the region's spread of pitch.
                let spread = if across { 0.25 } else { 1.0 };
                let pitch_deg = rules.pitch[0] + rules.pitch[1] * spread * h(3);
                // A deep house's roof no taller than 4.2 m (its back under a lower pitch, as an outrigger's).
                let rise = if across {
                    (rect.half[0] * pitch_deg.to_radians().tan()).min(4.2)
                } else {
                    rect.half[1] * pitch_deg.to_radians().tan()
                };
                let hipped_share = rules.hipped;
                let corners = rect.corners();
                let corners = if across {
                    vec![corners[1], corners[2], corners[3], corners[0]]
                } else {
                    corners.to_vec()
                };
                let roof = if across {
                    Roof::Gabled { rise }
                } else if block || h(4) < hipped_share {
                    Roof::Hipped { rise }
                } else {
                    Roof::Gabled { rise }
                };
                // The source's height is the roof's mean: the eaves stand half the rise below it.
                let eaves = if data.is_some() {
                    (walls_m - rise * 0.5).max(2.4)
                } else {
                    walls_m
                };
                (corners, roof, eaves, storeys, storey_m, true)
            } else {
                (ring, Roof::Flat, walls_m, storeys, storey_m, false)
            }
        }
    };
    if !matches!(
        kind,
        Kind::House | Kind::Terrace | Kind::Block | Kind::Tower | Kind::Office
    ) {
        detail = false;
    }
    let (mut wall, roof_colour) = colours(region, kind, period, pitched, h(5), h(6));
    if let Some(rgb) = footprint.colour.filter(|_| !period.is_past()) {
        wall = cube_entry(rgb);
    }
    Some(Building {
        ring,
        kind,
        roof,
        ground: [lo, mean, hi],
        height: height.max(2.2),
        base,
        storeys,
        storey_m,
        wall,
        roof_colour,
        seed,
        detail,
        bay,
        extras,
        period,
    })
}

/// A building's kind from a measured height, its area and its land's usage (`t` a hash in 0-1).
fn measured_kind(height: f32, area: f32, usage: Usage, t: f32) -> Kind {
    if height >= 36.0 {
        return if usage == Usage::Commercial {
            Kind::Office
        } else {
            Kind::Tower
        };
    }
    if usage == Usage::Industrial && area >= 60.0 && height < 16.0 {
        return if area < 250.0 {
            Kind::Warehouse
        } else if area >= 900.0 && t < 0.45 {
            Kind::Industrial
        } else {
            Kind::Warehouse
        };
    }
    if area >= 1500.0 && height < 13.0 {
        return if usage == Usage::Commercial {
            Kind::Warehouse
        } else {
            Kind::Industrial
        };
    }
    if area < 30.0 {
        Kind::Shed
    } else if area < 260.0 && height < 11.0 {
        Kind::House
    } else if height < 11.0 {
        Kind::Terrace
    } else if usage == Usage::Commercial {
        Kind::Office
    } else {
        Kind::Block
    }
}

/// A place of worship: its body (a steep gabled nave, the Mediterranean's low tile roof, a mosque's flat roof under
/// its dome) and what stands with it: a Christian church's tower at its west end (a spire in the north and west of
/// Europe and the Americas, a campanile's low pyramid and a dome over a large church in the Mediterranean), a mosque's
/// dome and its one, two or four minarets. A measured height (OSM's, the tallest part's) is the tower's top.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn church(
    ring: &[[f32; 2]],
    rect: &geometry::Rect,
    area: f32,
    rectangular: f32,
    data: Option<f32>,
    religion: Religion,
    region: Region,
    h: &dyn Fn(i64) -> f32,
    extras: &mut Vec<Extra>,
) -> (Vec<[f32; 2]>, Roof, f32, u8, f32, bool) {
    let short = 2.0 * rect.half[1];
    let eaves = (4.5 + 0.3 * short).clamp(5.0, 20.0);
    let rect_like = rectangular > 0.7 && short <= 34.0;
    let normal = [-rect.axis[1], rect.axis[0]];
    let at = |u: f32, v: f32| {
        [
            rect.centre[0] + rect.axis[0] * u + normal[0] * v,
            rect.centre[1] + rect.axis[1] * u + normal[1] * v,
        ]
    };
    match religion {
        Religion::Muslim => {
            let want = (0.36 * short).clamp(3.0, 20.0);
            let base = eaves.max(6.0 + 0.1 * short);
            let dome_colour = pick(&[22, 31, 21, 15, 30, 22], h(40));
            // Over the hall's middle, or where a concave footprint holds the most room; none on a footprint too thin.
            let held =
                |c: [f32; 2]| geometry::contains(ring, c).then(|| geometry::ring_distance(ring, c));
            let seat = match held(rect.centre) {
                Some(room) if room >= want * 0.8 => Some((rect.centre, want)),
                _ => geometry::inner_point(ring)
                    .filter(|(_, room)| *room >= 2.5)
                    .map(|(c, room)| (c, want.min(room))),
            };
            let radius = seat.map_or(0.0, |(_, r)| r);
            if let Some((centre, radius)) = seat {
                extras.push(Extra::Dome {
                    centre,
                    radius,
                    base,
                    colour: dome_colour,
                });
            }
            let count = if area < 600.0 {
                1
            } else if area < 3000.0 {
                2
            } else {
                4
            };
            let mr = 1.4 + 0.6 * h(41);
            let top = data
                .filter(|d| *d > base + radius)
                .unwrap_or((base + radius) * 1.7 + 8.0 + 14.0 * h(42))
                .min(90.0);
            let inset = |half: f32| (half - mr - 0.5).max(0.0);
            let (lu, lv) = (inset(rect.half[0]), inset(rect.half[1]));
            let corners = [at(-lu, -lv), at(-lu, lv), at(lu, -lv), at(lu, lv)];
            // A corner of the rectangle a concave footprint does not hold has no minaret.
            for c in corners
                .iter()
                .take(count)
                .filter(|c| held(**c).is_some_and(|room| room >= mr * 0.5))
            {
                extras.push(Extra::Minaret {
                    centre: *c,
                    radius: mr,
                    top,
                    colour: dome_colour,
                });
            }
            (ring.to_vec(), Roof::Flat, base, 1, base, false)
        }
        Religion::Christian => {
            let pitch: f32 = match region {
                Region::Mediterranean
                | Region::Arid
                | Region::West
                | Region::Malta
                | Region::Aegean
                | Region::NorthAfrica
                | Region::Nile => 20.0 + 8.0 * h(43),
                Region::Tropical
                | Region::EastAsia
                | Region::Taiwan
                | Region::Florida
                | Region::Hawaii
                | Region::Micronesia
                | Region::Atoll => 28.0 + 10.0 * h(43),
                _ => 42.0 + 10.0 * h(43),
            };
            let (body, roof) = if rect_like {
                let rise = rect.half[1] * pitch.to_radians().tan();
                (rect.corners().to_vec(), Roof::Gabled { rise })
            } else {
                (ring.to_vec(), Roof::Flat)
            };
            let ridge = eaves + roof.rise();
            // The west end (the altar's east); a church lying north and south takes its north end. On a footprint
            // that is no rectangle (a cross, a tower's outline merged with its nave's), the tower stands in from that
            // end as far as the footprint holds it.
            let half = (0.28 * short).clamp(2.2, 7.0);
            let west = if rect.axis[0].abs() >= 0.3 {
                -rect.axis[0].signum()
            } else {
                -rect.axis[1].signum()
            };
            let end = west * (rect.half[0] - half).max(0.0);
            let within = (0..6).map(|k| end * (1.0 - 0.2 * k as f32)).find(|&u| {
                let c = at(u, 0.0);
                geometry::contains(ring, c) && geometry::ring_distance(ring, c) >= half * 0.6
            });
            if area >= 120.0
                && let Some(u) = within
            {
                let top = data
                    .filter(|d| *d > ridge + 3.0)
                    .unwrap_or(ridge * (1.5 + 0.5 * h(44)) + 4.0)
                    .min(160.0);
                let cap = match region {
                    // A mission's campanile in the American West.
                    Region::Mediterranean
                    | Region::Arid
                    | Region::West
                    | Region::Malta
                    | Region::Aegean
                    | Region::NorthAfrica
                    | Region::Nile => Cap::Pyramid {
                        height: half * (0.7 + 0.4 * h(45)),
                        colour: pick(&[0, 16, 1], h(46)),
                    },
                    // An English parish church's square tower, battlemented, more often than a spire.
                    Region::British if h(49) < 0.55 => Cap::Flat,
                    Region::Nordic | Region::Iceland => Cap::Spire {
                        height: half * 2.0 * (1.6 + 1.2 * h(45)),
                        colour: pick(&[4, 12, 21, 12], h(46)),
                    },
                    // A white clapboard or stucco church's steeple.
                    Region::Florida | Region::Hawaii => Cap::Spire {
                        height: half * 2.0 * (1.2 + 1.0 * h(45)),
                        colour: pick(&[15, 18, 21], h(46)),
                    },
                    Region::Tropical | Region::Micronesia | Region::Atoll => Cap::Spire {
                        height: half * 2.0 * (1.0 + 0.8 * h(45)),
                        colour: pick(&[7, 4, 29], h(46)),
                    },
                    _ => Cap::Spire {
                        height: half * 2.0 * (1.8 + 1.4 * h(45)),
                        colour: pick(&[21, 22, 2, 12, 25, 21], h(46)),
                    },
                };
                extras.push(Extra::Tower {
                    centre: at(u, 0.0),
                    axis: rect.axis,
                    half,
                    top,
                    cap,
                });
                // A dome over a large Mediterranean church's crossing; over most Greek churches, small as they are
                // (blue, terracotta or white), and Malta's (lead and red).
                let crossing = at(-u * 0.35, 0.0);
                let radius = (0.32 * short).clamp(3.0, 14.0);
                let (least, share, colours): (f32, f32, &[u8]) = match region {
                    Region::Aegean => (150.0, 0.8, &[6, 25, 0, 15, 6]),
                    Region::Malta => (400.0, 0.7, &[22, 0, 21, 22]),
                    Region::Mediterranean | Region::Arid => (900.0, 0.6, &[22, 0, 21, 15]),
                    _ => (f32::INFINITY, 0.0, &[22]),
                };
                if area >= least
                    && h(48) < share
                    && geometry::contains(ring, crossing)
                    && geometry::ring_distance(ring, crossing) >= radius * 0.8
                {
                    extras.push(Extra::Dome {
                        centre: crossing,
                        radius,
                        base: ridge,
                        colour: pick(colours, h(47)),
                    });
                }
            }
            (body, roof, eaves, 1, eaves, rect_like)
        }
        _ => {
            // A synagogue, temple or the rest: a hipped roof, no tower (an East Asian temple's or shrine's steeper).
            let (body, roof) = if rect_like {
                let pitch: f32 = if matches!(region, Region::EastAsia | Region::Taiwan) {
                    36.0 + 6.0 * h(43)
                } else {
                    30.0
                };
                let rise = rect.half[1] * pitch.to_radians().tan();
                (rect.corners().to_vec(), Roof::Hipped { rise })
            } else {
                (ring.to_vec(), Roof::Flat)
            };
            let pitched = rect_like;
            (body, roof, eaves, 1, eaves, pitched)
        }
    }
}

/// A storey's height (m) for windows: an industrial shed's or warehouse's whole wall is two.
pub fn storey_height(kind: Kind) -> f32 {
    match kind {
        Kind::Shed => 2.6,
        Kind::House | Kind::Terrace => 2.8,
        Kind::Block | Kind::Tower => 3.1,
        Kind::Office => 3.6,
        Kind::Industrial | Kind::Warehouse => 4.5,
        _ => 3.0,
    }
}

/// A window bay's width (m) for a kind.
pub fn bay_m(kind: Kind) -> f32 {
    match kind {
        Kind::Shed => 3.5,
        Kind::House | Kind::Terrace => 3.0,
        Kind::Block | Kind::Tower => 3.3,
        Kind::Office => 3.0,
        Kind::Industrial => 6.0,
        Kind::Warehouse => 7.0,
        Kind::Church => 4.5,
        _ => 4.0,
    }
}

/// The most window rows a building's walls hold (the vertex's snorm8): 457 m of offices' storeys, 394 m of flats'.
pub const MAX_STOREYS: u8 = 127;

fn storeys_of(height: f32, storey_m: f32) -> u8 {
    (height / storey_m)
        .round()
        .clamp(1.0, f32::from(MAX_STOREYS)) as u8
}

/// The share of Taiwan's large compact outlines (1,300-4,000 m², built over, no height mapped) estimated as towers
/// ([`estimate`]). Of Taoyuan's and Zhongli's 1,366 such outlines in OSM (2026-10-29) 483 say nothing of what they
/// are, 287 are industrial, 136 apartments, 111 residential, 105 schools and the rest shops, offices, halls and
/// stations; 37 of the apartments' 40 with storeys mapped have nine or more, none of the 27 untyped ones has.
pub const TAIWAN_TOWER_SHARE: f32 = 0.3;

/// A building's kind and storeys without a measured height: from its area, its elongation (long over short side), the
/// share of the ground built over about it and its land's `usage`, varied by `t` (0-1). A detached house about 6 m, a
/// terrace 8 m, blocks 12-25 m in dense cores (offices on commercial land), warehouses and factories 8-10 m on
/// industrial land and large footprints.
pub fn estimate(
    area: f32,
    elongation: f32,
    density: f32,
    t: f32,
    region: Region,
    usage: Usage,
) -> (Kind, u8) {
    let up = |base: u8, spread: u8| base + (t * (f32::from(spread) + 0.999)) as u8;
    if usage == Usage::Industrial {
        return if area < 50.0 {
            (Kind::Shed, 1)
        } else if area < 300.0 {
            (Kind::Warehouse, 1)
        } else if area >= 900.0 && t < 0.4 {
            (Kind::Industrial, 2)
        } else {
            (Kind::Warehouse, 2)
        };
    }
    if area < 30.0 {
        return (Kind::Shed, 1);
    }
    let commercial = usage == Usage::Commercial;
    let block = |storeys: u8| {
        if commercial && storeys >= 3 {
            (Kind::Office, storeys)
        } else {
            (Kind::Block, storeys)
        }
    };
    if area < 220.0 {
        return match region {
            Region::Tropical | Region::Arid if density < 0.3 => (Kind::House, 1),
            Region::American if density < 0.35 => (Kind::House, up(1, 1)),
            // Brick and stone terraces of two storeys (three now and then) wherever the ground is built over; semis
            // and houses of two storeys about the towns, cottages of one in Scotland's and Ireland's country.
            Region::British | Region::Celtic | Region::NorthSea if density > 0.3 => {
                (Kind::Terrace, if t < 0.85 { 2 } else { 3 })
            }
            Region::British | Region::NorthSea => (Kind::House, if t < 0.88 { 2 } else { 1 }),
            Region::Celtic => (Kind::House, if t < 0.45 { 1 } else { 2 }),
            // Single-storey houses on wide lots, two storeys now and then.
            Region::Florida if density < 0.45 => (Kind::House, if t < 0.78 { 1 } else { 2 }),
            // Malta's terraced townhouses of two and three storeys.
            Region::Malta if density > 0.3 => (Kind::Terrace, up(2, 1)),
            // A narrow polykatoikia of four to six storeys in Athens' and Piraeus' streets; a house of one or two
            // elsewhere.
            Region::Aegean if density > 0.45 => block(up(4, 2)),
            Region::Aegean => (Kind::House, up(1, 1)),
            // A medina's houses of two and three storeys about their courts; a village's of one.
            Region::NorthAfrica if density > 0.45 => (Kind::House, up(2, 1)),
            Region::NorthAfrica => (Kind::House, if t < 0.7 { 1 } else { 2 }),
            // The delta's and the valley's narrow blocks of three to five storeys, red brick in concrete frames.
            Region::Nile if density > 0.25 => block(up(3, 2)),
            Region::Nile => (Kind::House, 2),
            // Hawaii's houses of one storey (two on two in five) close together; a walk-up where it is built over.
            Region::Hawaii if density < 0.55 => (Kind::House, if t < 0.62 { 1 } else { 2 }),
            Region::Hawaii => block(up(3, 1)),
            // Guam's and the atolls' houses of one storey (two now and then).
            Region::Micronesia => (Kind::House, if t < 0.85 { 1 } else { 2 }),
            Region::Atoll => (Kind::House, if t < 0.92 { 1 } else { 2 }),
            // Ranch houses of one storey, two-storey houses; townhouses only where the ground is built over.
            Region::West if density < 0.5 => (Kind::House, if t < 0.6 { 1 } else { 2 }),
            // Taiwan's townhouses (透天厝, 街屋), narrow and deep in their rows: four and five storeys wherever the
            // ground is built over, three and four about the towns.
            Region::Taiwan if density > 0.3 => (Kind::Terrace, up(4, 1)),
            Region::Taiwan => (Kind::Terrace, up(3, 1)),
            // Small houses close together: two storeys, now and then one or three.
            Region::EastAsia => (
                Kind::House,
                if t < 0.15 {
                    1
                } else if t < 0.94 {
                    2
                } else {
                    3
                },
            ),
            _ if density > 0.35 => (Kind::Terrace, up(2, 1)),
            _ => (Kind::House, up(1, 1)),
        };
    }
    match region {
        // A suburb's big houses and garden apartments; blocks only downtown.
        Region::West if area < 1300.0 => {
            return if density > 0.55 {
                block(up(3, 2))
            } else if area < 450.0 && elongation < 2.2 {
                (Kind::House, up(1, 1))
            } else {
                (Kind::Terrace, up(2, 1))
            };
        }
        Region::West if density > 0.55 && elongation < 3.0 => {
            return block(up(4, 4).min(MAX_ESTIMATED_STOREYS));
        }
        // Flat concrete mid-rise blocks wherever the ground is built over; a two-storey apartment house elsewhere.
        // Athens' polykatoikia of four to seven storeys; a suburb's houses of two and three.
        Region::Aegean if area < 1300.0 => {
            return if density > 0.3 {
                block(up(4, 3))
            } else {
                (Kind::House, up(2, 1))
            };
        }
        // Cairo's and Alexandria's blocks of four to eight storeys, the delta's of three to six.
        Region::Nile if area < 1300.0 => {
            return if density > 0.3 {
                block(up(4, 4))
            } else {
                block(up(3, 3))
            };
        }
        // Honolulu's walk-ups of three to six storeys; a big house or townhouses about the suburbs.
        Region::Hawaii if area < 1300.0 => {
            return if density > 0.45 {
                block(up(3, 3))
            } else if area < 450.0 && elongation < 2.2 {
                (Kind::House, up(1, 1))
            } else {
                (Kind::Terrace, up(2, 1))
            };
        }
        // Guam's two- and three-storey concrete blocks in Tamuning and Hagåtña; an atoll's meeting house, school or
        // store of one or two.
        Region::Micronesia if area < 1300.0 => {
            return if density > 0.4 {
                block(up(2, 1))
            } else {
                (Kind::House, up(1, 1))
            };
        }
        Region::Atoll if area < 1300.0 => {
            return if t < 0.7 { (Kind::House, 1) } else { block(2) };
        }
        // Taiwan's walk-ups (公寓) and rows of townhouses of four and five storeys where the ground is built over (a
        // row's outline, long for its depth, its townhouses), three and four about the towns; shops in their ground
        // floors on commercial land, not offices.
        Region::Taiwan if area < 1300.0 => {
            return if density <= 0.3 {
                (Kind::Terrace, up(3, 1))
            } else if elongation > 1.8 {
                (Kind::Terrace, up(4, 1))
            } else {
                (Kind::Block, up(4, 1))
            };
        }
        // A long row's outline its townhouses; a school's wings.
        Region::Taiwan if elongation > 3.0 && usage != Usage::Civic => {
            return (
                Kind::Terrace,
                if density > 0.3 { up(4, 1) } else { up(3, 1) },
            );
        }
        // Taiwan's towers (大樓) of 12 to 25 storeys where the ground is built over (and about a residential
        // district's edge), shops below and flats above, an office tower on a large commercial footprint: on three in
        // ten of these outlines ([`TAIWAN_TOWER_SHARE`]). The rest are a block of four or five (cut into its walk-ups,
        // `parcels`): OSM traces many of a dense core's blocks of townhouses as one outline (Zhongli's by its
        // station), and a market's, a store's or a car park's is low; a school's or a hospital's is a block.
        Region::Taiwan
            if area < 4000.0
                && elongation < 2.5
                && (density > 0.3 || (usage == Usage::Residential && density > 0.15)) =>
        {
            let share = if usage == Usage::Civic {
                0.0
            } else {
                TAIWAN_TOWER_SHARE
            };
            if t < share {
                let storeys = 12 + (t / share * 13.999) as u8;
                return if commercial && area >= 2500.0 {
                    (Kind::Office, storeys)
                } else {
                    (Kind::Tower, storeys)
                };
            }
            return (Kind::Block, 4 + ((t - share) / (1.0 - share) * 1.999) as u8);
        }
        // A market's, a department store's or an estate's block of five to eight under one outline; a school, an
        // estate's walk-ups; a village's factories and stores.
        Region::Taiwan if density > 0.3 => return block(up(5, 3).min(MAX_ESTIMATED_STOREYS)),
        Region::Taiwan => {
            return match usage {
                Usage::Residential | Usage::Civic => (Kind::Block, up(3, 2)),
                Usage::Commercial => (Kind::Warehouse, 2),
                _ => (Kind::Industrial, 2),
            };
        }
        Region::EastAsia if area < 1300.0 => {
            return if density > 0.3 {
                block(up(3, 3))
            } else {
                (Kind::Terrace, up(2, 1))
            };
        }
        Region::EastAsia if density > 0.3 && elongation < 3.5 => {
            return block(up(5, 3).min(MAX_ESTIMATED_STOREYS));
        }
        Region::West | Region::EastAsia => {
            return match usage {
                Usage::Residential | Usage::Civic => (Kind::Block, up(2, 1)),
                Usage::Commercial => (Kind::Warehouse, 2),
                _ => (Kind::Industrial, 2),
            };
        }
        _ => {}
    }
    if area < 1300.0 {
        if elongation > 2.6 && density < 0.45 {
            return (Kind::Terrace, up(2, 1));
        }
        return if density > 0.45 {
            block(up(4, 2))
        } else if density > 0.28 {
            block(up(3, 2))
        } else {
            (Kind::Terrace, up(2, 1))
        };
    }
    if density > 0.4 && elongation < 3.0 {
        return block(up(5, 3).min(MAX_ESTIMATED_STOREYS));
    }
    match usage {
        // A residential estate's long block, a school.
        Usage::Residential | Usage::Civic => (Kind::Block, up(2, 2)),
        // A shop's big box.
        Usage::Commercial => (Kind::Warehouse, 2),
        // Large and low: a warehouse or factory, 8-10 m (two of its 4.5 m "storeys").
        _ => (Kind::Industrial, 2),
    }
}

/// The share of a chunk's ground built over, on a grid of `cells`² squares of the chunk, each averaged with its
/// neighbours: what [`plan_building`] reads as `density` at a point (`DensityGrid::at`).
#[derive(Clone, Debug)]
pub struct DensityGrid {
    cells: usize,
    share: Vec<f32>,
}

impl DensityGrid {
    /// Squares of 100 m over the chunk.
    pub const CELLS: usize = 10;

    pub fn new(footprints: &[Footprint]) -> Self {
        let cells = Self::CELLS;
        let side = CHUNK_M as f32 / cells as f32;
        let mut built = vec![0f32; cells * cells];
        for f in footprints {
            // A raised part stands over ground its building's other parts already cover.
            if f.ring.len() < 3 || f.raised() {
                continue;
            }
            let c = geometry::centroid(&f.ring);
            let (i, j) = ((c[0] / side).floor(), (c[1] / side).floor());
            if (0.0..cells as f32).contains(&i) && (0.0..cells as f32).contains(&j) {
                built[j as usize * cells + i as usize] += geometry::area(&f.ring);
            }
        }
        let mut share = vec![0f32; cells * cells];
        for j in 0..cells {
            for i in 0..cells {
                let (mut sum, mut n) = (0.0, 0.0);
                for dj in -1i32..=1 {
                    for di in -1i32..=1 {
                        let (x, z) = (i as i32 + di, j as i32 + dj);
                        if x >= 0 && z >= 0 && (x as usize) < cells && (z as usize) < cells {
                            let w = if di == 0 && dj == 0 { 2.0 } else { 1.0 };
                            sum += w * built[z as usize * cells + x as usize];
                            n += w;
                        }
                    }
                }
                share[j * cells + i] = (sum / n / (side * side)).min(1.0);
            }
        }
        Self { cells, share }
    }

    /// The share built over about chunk-local point `p`.
    pub fn at(&self, p: [f32; 2]) -> f32 {
        let side = CHUNK_M as f32 / self.cells as f32;
        let i = ((p[0] / side).floor() as i64).clamp(0, self.cells as i64 - 1) as usize;
        let j = ((p[1] / side).floor() as i64).clamp(0, self.cells as i64 - 1) as usize;
        self.share[j * self.cells + i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, z: f32, side: f32) -> Footprint {
        Footprint {
            ring: vec![[x, z], [x + side, z], [x + side, z + side], [x, z + side]],
            ..Footprint::default()
        }
    }

    fn rect(x: f32, z: f32, w: f32, d: f32) -> Footprint {
        Footprint {
            ring: vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]],
            ..Footprint::default()
        }
    }

    fn circle(x: f32, z: f32, radius: f32) -> Footprint {
        Footprint {
            ring: geometry::regular([x, z], radius, 14, 0.1),
            ..Footprint::default()
        }
    }

    fn plain() -> Setting {
        Setting::default()
    }

    fn on(usage: Usage) -> Setting {
        Setting {
            usage,
            ..Setting::default()
        }
    }

    fn with_poi(poi: PoiKind) -> Setting {
        Setting {
            poi: Some(poi),
            ..Setting::default()
        }
    }

    #[test]
    fn the_tallest_towers_keep_their_heights() {
        let ground = |_: [f32; 2]| 5.0;
        let plan = |height: f32| {
            let tower = Footprint {
                height: Some(height),
                ..square(400.0, 400.0, 60.0)
            };
            plan_building(
                &tower,
                [0, 0],
                0.6,
                Region::Temperate,
                Period::Today,
                &on(Usage::Commercial),
                &ground,
            )
            .unwrap()
        };
        // Taipei 101's roof and spire, and the Burj Khalifa's 828 m: taken; past the tallest, estimated.
        for height in [449.0, 508.0, 828.0] {
            let b = plan(height);
            assert_eq!((b.kind, b.height), (Kind::Office, height));
            assert!(b.storeys <= MAX_STOREYS);
        }
        assert!(plan(900.0).height < 100.0);
        // A storey of offices' window rows for each of its 449 m.
        assert_eq!(plan(449.0).storeys, 125);
    }

    #[test]
    fn a_part_takes_its_building_s_hashes_and_ground_and_stands_from_its_base() {
        let ground = |p: [f32; 2]| 5.0 + 0.1 * (p[0] - 400.0);
        let part = |x: f32, side: f32, base: f32, top: f32| Footprint {
            height: Some(top),
            part: Some(Part {
                base,
                middle: [420.0, 420.0],
            }),
            ..square(x, x, side)
        };
        let plan = |f: &Footprint| {
            plan_building(
                f,
                [0, 0],
                0.6,
                Region::Temperate,
                Period::Today,
                &with_poi(PoiKind::Worship(Religion::Christian)),
                &ground,
            )
            .unwrap()
        };
        let shaft = plan(&part(400.0, 40.0, 0.0, 120.0));
        let crown = plan(&part(410.0, 20.0, 120.0, 160.0));
        // Not a church's made tower on each part: as mapped, under a flat top.
        assert_eq!((shaft.kind, crown.kind), (Kind::Tower, Kind::Tower));
        assert!(shaft.extras.is_empty() && crown.extras.is_empty());
        assert_eq!(crown.roof, Roof::Flat);
        assert_eq!(
            (shaft.wall, shaft.seed, shaft.bay, shaft.ground[1]),
            (crown.wall, crown.seed, crown.bay, crown.ground[1])
        );
        assert_eq!(shaft.ground[1], ground([420.0, 420.0]));
        assert_eq!((shaft.base, crown.base), (0.0, 120.0));
        // A mast too narrow for rooms: no window rows up it. A part over the period's cap: none of it.
        assert_eq!(plan(&part(419.0, 2.0, 160.0, 200.0)).storeys, 1);
        let capped = Setting {
            cap: Some(100.0),
            ..Setting::default()
        };
        let then = |f: &Footprint| {
            plan_building(
                f,
                [0, 0],
                0.6,
                Region::Temperate,
                Period::Year(1942),
                &capped,
                &ground,
            )
        };
        assert!(then(&part(410.0, 20.0, 120.0, 160.0)).is_none());
        assert_eq!(then(&part(400.0, 40.0, 0.0, 120.0)).unwrap().height, 100.0);
    }

    #[test]
    fn an_industrial_part_stands_under_a_flat_top() {
        // A 20 by 40 m part of a works from 10 to 15 m: its top at 15 m as mapped, no gable over it into a part above.
        let ground = |_: [f32; 2]| 5.0;
        let part = Footprint {
            height: Some(15.0),
            part: Some(Part {
                base: 10.0,
                middle: [410.0, 420.0],
            }),
            ..rect(400.0, 400.0, 20.0, 40.0)
        };
        let plan = |f: &Footprint| {
            plan_building(
                f,
                [0, 0],
                0.6,
                Region::Temperate,
                Period::Today,
                &on(Usage::Industrial),
                &ground,
            )
            .unwrap()
        };
        let b = plan(&part);
        assert_eq!(b.kind, Kind::Warehouse);
        assert_eq!((b.roof, b.height, b.base), (Roof::Flat, 15.0, 10.0));
        // The same footprint whole, on the ground: a warehouse's gable.
        let whole = plan(&Footprint {
            part: None,
            ..part.clone()
        });
        assert_eq!(whole.kind, Kind::Warehouse);
        assert!(
            matches!(whole.roof, Roof::Gabled { .. }),
            "{:?}",
            whole.roof
        );
    }

    #[test]
    fn a_capped_building_keeps_the_kind_its_height_gives() {
        let ground = |_: [f32; 2]| 5.0;
        let capped = |usage: Usage, cap: f32| Setting {
            usage,
            cap: Some(cap),
            ..Setting::default()
        };
        let past = Period::Year(1942);
        // A round tower 40 m across and 60 m tall: at a 25 m cap a block, not an oil tank.
        let tower = Footprint {
            height: Some(60.0),
            ..circle(500.0, 500.0, 20.0)
        };
        let setting = capped(Usage::Unknown, 25.0);
        let b = plan_building(
            &tower,
            [0, 0],
            0.3,
            Region::Temperate,
            past,
            &setting,
            &ground,
        )
        .unwrap();
        assert_eq!(b.kind, Kind::Block);
        assert!(b.height <= 25.5, "{}", b.height);
        // An office of 2,000 m² measured at 20 m: at the tropics' 12 m an office still, not a shop's big box.
        let office = Footprint {
            height: Some(20.0),
            ..square(100.0, 100.0, 45.0)
        };
        let setting = capped(Usage::Commercial, 12.0);
        let b = plan_building(
            &office,
            [0, 0],
            0.3,
            Region::Tropical,
            past,
            &setting,
            &ground,
        )
        .unwrap();
        assert_eq!(b.kind, Kind::Office);
        assert!(b.height <= 12.5, "{}", b.height);
        // Today a cap is not read.
        let today = Period::Today;
        let b = plan_building(
            &office,
            [0, 0],
            0.3,
            Region::Tropical,
            today,
            &setting,
            &ground,
        )
        .unwrap();
        assert!(b.height > 19.0, "{}", b.height);
    }

    #[test]
    fn heights_follow_the_data_when_sane_and_the_footprint_otherwise() {
        let flat = |_: [f32; 2]| 10.0f32;
        let t = Region::Temperate;
        // A 10 m square house in the suburbs: one or two storeys under a pitched roof.
        let house = plan_building(
            &square(100.0, 100.0, 10.0),
            [0, 0],
            0.1,
            t,
            Period::Today,
            &plain(),
            &flat,
        )
        .unwrap();
        assert_eq!(house.kind, Kind::House);
        assert!(matches!(
            house.roof,
            Roof::Gabled { .. } | Roof::Hipped { .. }
        ));
        assert!((3.0..7.0).contains(&house.height), "{}", house.height);
        assert_eq!(house.ground, [10.0, 10.0, 10.0]);
        // A 40 m square in a dense core: a block of 5-8 storeys.
        let block = plan_building(
            &square(100.0, 100.0, 40.0),
            [0, 0],
            0.6,
            t,
            Period::Today,
            &plain(),
            &flat,
        )
        .unwrap();
        assert!(
            matches!(block.kind, Kind::Block | Kind::Tower),
            "{:?}",
            block.kind
        );
        assert!(block.height >= 15.0, "{}", block.height);
        // The same square alone: a warehouse.
        let shed = plan_building(
            &square(100.0, 100.0, 40.0),
            [0, 0],
            0.05,
            t,
            Period::Today,
            &plain(),
            &flat,
        )
        .unwrap();
        assert_eq!(shed.kind, Kind::Industrial);
        assert!((8.0..=10.0).contains(&shed.height));
        // Measured: a 90 m tower is a tower; a 0.5 m reading is ignored.
        let mut tall = square(0.0, 0.0, 30.0);
        tall.height = Some(90.0);
        let tower = plan_building(
            &tall,
            [0, 0],
            0.5,
            Region::Tropical,
            Period::Today,
            &plain(),
            &flat,
        )
        .unwrap();
        assert_eq!(tower.kind, Kind::Tower);
        assert_eq!(tower.height, 90.0);
        assert_eq!(tower.storeys, 29);
        let mut low = square(0.0, 0.0, 10.0);
        low.height = Some(0.5);
        assert!(
            plan_building(
                &low,
                [0, 0],
                0.1,
                Region::Nordic,
                Period::Today,
                &plain(),
                &flat
            )
            .unwrap()
            .height
                > 2.5
        );
        // In the sea: none; on a pier, on its deck.
        let sea = |_: [f32; 2]| -3.0f32;
        assert!(
            plan_building(
                &square(0.0, 0.0, 10.0),
                [0, 0],
                0.1,
                t,
                Period::Today,
                &plain(),
                &sea
            )
            .is_none()
        );
        let pier = Setting {
            pier: Some(2.4),
            ..Setting::default()
        };
        let wharf = plan_building(
            &square(0.0, 0.0, 10.0),
            [0, 0],
            0.1,
            t,
            Period::Today,
            &pier,
            &sea,
        )
        .unwrap();
        assert_eq!(wharf.ground, [2.4, 2.4, 2.4]);
    }

    #[test]
    fn the_map_types_its_buildings() {
        let flat = |_: [f32; 2]| 10.0f32;
        let t = Region::Temperate;
        // On industrial land a large footprint is a warehouse or a factory, never a house or block.
        for k in 0..12 {
            let fp = rect(100.0 + 70.0 * k as f32, 100.0, 50.0, 30.0);
            let b = plan_building(
                &fp,
                [0, 0],
                0.5,
                t,
                Period::Today,
                &on(Usage::Industrial),
                &flat,
            )
            .unwrap();
            assert!(
                matches!(b.kind, Kind::Warehouse | Kind::Industrial),
                "{:?}",
                b.kind
            );
            assert!((6.0..=12.0).contains(&b.height), "{}", b.height);
        }
        // Factories now and then wear chimneys over their roofs, within the footprint.
        let mut chimneys = 0;
        for k in 0..60 {
            let fp = rect(
                10.0 + 15.0 * (k % 10) as f32,
                10.0 + 15.0 * (k / 10) as f32,
                60.0,
                40.0,
            );
            let b = plan_building(
                &fp,
                [k, 0],
                0.1,
                t,
                Period::Today,
                &on(Usage::Industrial),
                &flat,
            )
            .unwrap();
            for e in &b.extras {
                if let Extra::Chimney { centre, top, .. } = e {
                    assert_eq!(b.kind, Kind::Industrial);
                    assert!(geometry::contains(&fp.ring, *centre));
                    assert!(*top > b.height + 10.0);
                    chimneys += 1;
                }
            }
        }
        assert!(chimneys > 0);
        // A place of worship's footprint is a church with a tower at its west end and a spire in the north of Europe;
        // a dome and a pyramid-capped campanile in the Mediterranean; a mosque's dome and minarets.
        let nave = rect(100.0, 100.0, 36.0, 16.0);
        let church = plan_building(
            &nave,
            [0, 0],
            0.3,
            t,
            Period::Today,
            &with_poi(PoiKind::Worship(Religion::Christian)),
            &flat,
        )
        .unwrap();
        assert_eq!(church.kind, Kind::Church);
        assert!(matches!(church.roof, Roof::Gabled { .. }));
        let Some(Extra::Tower {
            centre, top, cap, ..
        }) = church.extras.first().copied()
        else {
            panic!("no tower: {:?}", church.extras);
        };
        assert!(
            centre[0] < 118.0 - 5.0,
            "the tower stands at the west end: {centre:?}"
        );
        assert!(top > church.height + church.roof.rise());
        assert!(matches!(cap, Cap::Spire { .. }));
        let south: Vec<Building> = (0..8)
            .map(|k| {
                plan_building(
                    &rect(100.0 + 60.0 * k as f32, 100.0, 50.0, 24.0),
                    [0, 0],
                    0.3,
                    Region::Mediterranean,
                    Period::Today,
                    &with_poi(PoiKind::Worship(Religion::Christian)),
                    &flat,
                )
                .unwrap()
            })
            .collect();
        assert!(south.iter().all(|b| b.extras.iter().any(|e| matches!(
            e,
            Extra::Tower {
                cap: Cap::Pyramid { .. },
                ..
            }
        ))));
        let domed = south
            .iter()
            .filter(|b| b.extras.iter().any(|e| matches!(e, Extra::Dome { .. })))
            .count();
        assert!((1..8).contains(&domed), "{domed} of 8 domed");
        let mosque = plan_building(
            &rect(100.0, 100.0, 40.0, 40.0),
            [0, 0],
            0.3,
            Region::Mediterranean,
            Period::Today,
            &with_poi(PoiKind::Worship(Religion::Muslim)),
            &flat,
        )
        .unwrap();
        assert_eq!(mosque.kind, Kind::Church);
        assert!(
            mosque
                .extras
                .iter()
                .any(|e| matches!(e, Extra::Dome { .. }))
        );
        let minarets = mosque
            .extras
            .iter()
            .filter(|e| matches!(e, Extra::Minaret { .. }))
            .count();
        assert_eq!(minarets, 2);
        // An L-shaped mosque (40 m arms 10 m wide): its dome and minarets on the footprint, not over the missing corner.
        let l = Footprint {
            ring: vec![
                [100.0, 100.0],
                [140.0, 100.0],
                [140.0, 110.0],
                [110.0, 110.0],
                [110.0, 140.0],
                [100.0, 140.0],
            ],
            ..Footprint::default()
        };
        let bent = plan_building(
            &l,
            [0, 0],
            0.3,
            Region::Mediterranean,
            Period::Today,
            &with_poi(PoiKind::Worship(Religion::Muslim)),
            &flat,
        )
        .unwrap();
        for e in &bent.extras {
            match *e {
                Extra::Dome { centre, radius, .. } => {
                    assert!(geometry::contains(&l.ring, centre), "{centre:?}");
                    assert!(geometry::ring_distance(&l.ring, centre) >= radius * 0.99);
                }
                Extra::Minaret { centre, .. } => assert!(geometry::contains(&l.ring, centre)),
                _ => {}
            }
        }
        // Measured tall and thin on industrial land: a stack, not a tank.
        let mut stack = circle(500.0, 500.0, 5.0);
        stack.height = Some(29.0);
        let planned = plan_building(
            &stack,
            [0, 0],
            0.1,
            t,
            Period::Today,
            &on(Usage::Industrial),
            &flat,
        )
        .unwrap();
        assert_eq!(planned.kind, Kind::Chimney);
        // A measured height (OSM's tower) is the tower's top: Hallgrímskirkja's 74.5 m.
        let mut tall = nave.clone();
        tall.height = Some(74.5);
        let landmark = plan_building(
            &tall,
            [0, 0],
            0.3,
            Region::Nordic,
            Period::Today,
            &with_poi(PoiKind::Worship(Religion::Christian)),
            &flat,
        )
        .unwrap();
        assert!(
            landmark
                .extras
                .iter()
                .any(|e| matches!(e, Extra::Tower { top, .. } if *top == 74.5))
        );
        // A round footprint on industrial land is an oil tank; the same in a residential street stays a building.
        let tank = plan_building(
            &circle(300.0, 300.0, 20.0),
            [0, 0],
            0.2,
            t,
            Period::Today,
            &on(Usage::Industrial),
            &flat,
        )
        .unwrap();
        assert_eq!(tank.kind, Kind::Tank);
        assert!((6.0..=25.0).contains(&tank.height));
        assert!(tank.ring.len() >= 16);
        let round = plan_building(
            &circle(300.0, 300.0, 20.0),
            [0, 0],
            0.2,
            t,
            Period::Today,
            &on(Usage::Residential),
            &flat,
        )
        .unwrap();
        assert_ne!(round.kind, Kind::Tank);
        // A lighthouse's point makes its small footprint a lighthouse.
        let light = plan_building(
            &circle(50.0, 50.0, 3.0),
            [0, 0],
            0.0,
            t,
            Period::Today,
            &with_poi(PoiKind::Lighthouse),
            &flat,
        )
        .unwrap();
        assert_eq!(light.kind, Kind::Lighthouse);
        assert!(
            light
                .extras
                .iter()
                .any(|e| matches!(e, Extra::Light { .. }))
        );
        // On commercial land blocks are offices.
        let office = plan_building(
            &square(100.0, 100.0, 30.0),
            [0, 0],
            0.6,
            t,
            Period::Today,
            &on(Usage::Commercial),
            &flat,
        )
        .unwrap();
        assert_eq!(office.kind, Kind::Office);
        // OSM's colour paints the walls.
        let mut red = square(100.0, 100.0, 10.0);
        red.colour = Some([255, 0, 0]);
        let painted = plan_building(&red, [0, 0], 0.1, t, Period::Today, &plain(), &flat).unwrap();
        assert_eq!(painted.wall, cube_entry([255, 0, 0]));
    }

    #[test]
    fn roofs_follow_the_region() {
        // Houses' roofs by region: terracotta around the Mediterranean, slate and dark tiles in the north, corrugated
        // iron in the tropics, flat concrete in a dry country.
        let flat = |_: [f32; 2]| 10.0f32;
        let roofs = |region: Region| -> Vec<Building> {
            (0..80)
                .filter_map(|k| {
                    let fp = rect(
                        20.0 + 12.0 * (k % 20) as f32,
                        20.0 + 15.0 * (k / 20) as f32,
                        10.0,
                        8.0,
                    );
                    plan_building(
                        &fp,
                        [k, 1],
                        0.1,
                        region,
                        Period::Today,
                        &on(Usage::Residential),
                        &flat,
                    )
                })
                .collect()
        };
        let share = |bs: &[Building], entries: &[u8], pitched: bool| {
            let n = bs
                .iter()
                .filter(|b| entries.contains(&b.roof_colour) && (b.roof != Roof::Flat) == pitched)
                .count();
            n as f32 / bs.len() as f32
        };
        let terracotta = [0, 1, 16, 17, 23, 28];
        let dark = [2, 25, 26, 1, 23, 13];
        let tin = [7, 8, 29, 4, 6, 5];
        assert!(share(&roofs(Region::Mediterranean), &terracotta, true) > 0.85);
        assert!(share(&roofs(Region::Temperate), &dark, true) > 0.6);
        assert!(share(&roofs(Region::Tropical), &tin, true) > 0.75);
        let arid = roofs(Region::Arid);
        assert!(
            arid.iter().filter(|b| b.roof == Roof::Flat).count() as f32 / arid.len() as f32 > 0.7
        );
        // The American West: grey and brown shingles, a flat gravel or tar roof on some, little terracotta; East
        // Asia: kawara, dark glazed tile and sheet metal, no terracotta.
        let west = roofs(Region::West);
        let shingle = [18, 19, 20, 3];
        assert!(
            share(&west, &shingle, true) > 0.55,
            "{}",
            share(&west, &shingle, true)
        );
        let flat_share =
            west.iter().filter(|b| b.roof == Roof::Flat).count() as f32 / west.len() as f32;
        assert!((0.08..0.4).contains(&flat_share), "{flat_share}");
        assert!(share(&west, &terracotta, true) < 0.15);
        let east = roofs(Region::EastAsia);
        let japanese = [KAWARA, 26, 12, 25, 22, 7, 6, 5, 27];
        assert!(
            share(&east, &japanese, true) > 0.85,
            "{}",
            share(&east, &japanese, true)
        );
        assert_eq!(share(&east, &terracotta, true), 0.0);
        assert!(
            east.iter()
                .filter(|b| [KAWARA, 26, 12].contains(&b.roof_colour))
                .count()
                > east.len() / 3
        );
        // Britain: brick under slate and dark tile, no orange terracotta; Scotland and Ireland: harl, render and
        // stone, no brick; the North Sea: brick under orange and red pantiles and black glazed tile; Florida: light
        // stucco under hipped shingle and barrel tile.
        let walls_share = |bs: &[Building], entries: &[u8]| {
            bs.iter().filter(|b| entries.contains(&b.wall)).count() as f32 / bs.len() as f32
        };
        let brick = [4, 5, 6, palette::KLINKER];
        let british = roofs(Region::British);
        let slate = [2, 25, 26, 23, 13, 1];
        assert!(
            share(&british, &slate, true) > 0.85,
            "{}",
            share(&british, &slate, true)
        );
        assert_eq!(share(&british, &[0, 16, 17, 28], true), 0.0);
        assert!(
            walls_share(&british, &brick) > 0.5,
            "{}",
            walls_share(&british, &brick)
        );
        let celtic = roofs(Region::Celtic);
        assert!(share(&celtic, &[2, 25, 26, 3], true) > 0.9);
        assert_eq!(walls_share(&celtic, &brick), 0.0);
        assert!(walls_share(&celtic, &[palette::HARL, 30, palette::RED_SANDSTONE]) > 0.3);
        let north = roofs(Region::NorthSea);
        let pantile = [16, 0, 23, 1];
        assert!(
            share(&north, &pantile, true) > 0.4,
            "{}",
            share(&north, &pantile, true)
        );
        assert!(
            walls_share(&north, &brick) > 0.65,
            "{}",
            walls_share(&north, &brick)
        );
        let florida = roofs(Region::Florida);
        let hipped = florida
            .iter()
            .filter(|b| matches!(b.roof, Roof::Hipped { .. }))
            .count() as f32
            / florida.len() as f32;
        assert!(hipped > 0.45, "{hipped}");
        assert_eq!(walls_share(&florida, &brick), 0.0);
        // Malta: flat roofs over honey limestone; Greece: half flat, half terracotta; North Africa: flat over white,
        // sand and ochre, no brick; the Nile: flat over brick and render. A warm country's flat roofs pale: no tar or
        // asphalt.
        let flat_share = |bs: &[Building]| {
            bs.iter().filter(|b| b.roof == Roof::Flat).count() as f32 / bs.len() as f32
        };
        let malta = roofs(Region::Malta);
        assert!(flat_share(&malta) > 0.85, "{}", flat_share(&malta));
        assert!(walls_share(&malta, &[palette::HONEY]) > 0.3);
        assert!(share(&malta, &[palette::SCREED], false) > 0.3);
        let aegean = roofs(Region::Aegean);
        assert!(
            (0.35..0.75).contains(&flat_share(&aegean)),
            "{}",
            flat_share(&aegean)
        );
        assert!(share(&aegean, &terracotta, true) > 0.2);
        let africa = roofs(Region::NorthAfrica);
        assert!(flat_share(&africa) > 0.75, "{}", flat_share(&africa));
        assert_eq!(walls_share(&africa, &brick), 0.0);
        let nile = roofs(Region::Nile);
        assert!(flat_share(&nile) > 0.9, "{}", flat_share(&nile));
        assert!(walls_share(&nile, &brick) > 0.15);
        for region in [
            Region::Malta,
            Region::Aegean,
            Region::NorthAfrica,
            Region::Mediterranean,
            Region::Arid,
        ] {
            assert_eq!(share(&roofs(region), &[3, 11], false), 0.0, "{region:?}");
        }
        // Hawaii: pitched and mostly hipped, shingle and iron, never terracotta; Guam's flat concrete; an atoll's tin and
        // thatch.
        let hawaii = roofs(Region::Hawaii);
        let hipped = hawaii
            .iter()
            .filter(|b| matches!(b.roof, Roof::Hipped { .. }))
            .count() as f32
            / hawaii.len() as f32;
        assert!(hipped > 0.45, "{hipped}");
        assert!(share(&hawaii, &[18, 19, 20, 7, 29, 5, 4, 27], true) > 0.7);
        assert_eq!(share(&hawaii, &[0, 1, 16, 17, 23], true), 0.0);
        let micronesia = roofs(Region::Micronesia);
        assert!(
            (0.5..0.8).contains(&flat_share(&micronesia)),
            "{}",
            flat_share(&micronesia)
        );
        assert_eq!(walls_share(&micronesia, &brick), 0.0);
        let atoll = roofs(Region::Atoll);
        assert!(
            share(&atoll, &tin, true) > 0.6,
            "{}",
            share(&atoll, &tin, true)
        );
        assert!(share(&atoll, &[palette::THATCH], true) > 0.05);
        for region in [Region::Hawaii, Region::Micronesia, Region::Atoll] {
            assert_eq!(share(&roofs(region), &[3, 11], false), 0.0, "{region:?}");
        }
        // Not one house repeated: walls, roofs and seeds vary.
        let med = roofs(Region::Mediterranean);
        let walls: std::collections::HashSet<u8> = med.iter().map(|b| b.wall).collect();
        let seeds: std::collections::HashSet<u8> = med.iter().map(|b| b.seed).collect();
        assert!(
            walls.len() >= 6 && seeds.len() >= 60,
            "{} walls, {} seeds",
            walls.len(),
            seeds.len()
        );
        assert!(med.iter().any(|b| b.detail) && med.iter().any(|b| !b.detail));
    }

    #[test]
    fn taiwans_estimates_are_townhouses_walk_ups_and_towers() {
        let est = |area: f32, elongation: f32, density: f32, usage: Usage| {
            (0..40)
                .map(|k| {
                    estimate(
                        area,
                        elongation,
                        density,
                        k as f32 / 40.0,
                        Region::Taiwan,
                        usage,
                    )
                })
                .collect::<Vec<_>>()
        };
        let homes = Usage::Residential;
        // A dense street's footprints: townhouses and walk-ups of four and five storeys, both heights seen.
        for (area, elongation) in [(80.0, 2.5), (150.0, 1.4), (400.0, 1.3), (700.0, 2.4)] {
            let all = est(area, elongation, 0.45, homes);
            assert!(
                all.iter()
                    .all(|(k, s)| matches!(k, Kind::Terrace | Kind::Block) && (4..=5).contains(s)),
                "{area} {all:?}"
            );
            assert!(all.iter().any(|b| b.1 == 4) && all.iter().any(|b| b.1 == 5));
        }
        // Thinner ground: three and four.
        assert!(
            est(100.0, 2.0, 0.2, homes)
                .iter()
                .all(|(k, s)| *k == Kind::Terrace && (3..=4).contains(s))
        );
        // Over 1,300 m² where it is built over: towers of 12 to 25 on three in ten, offices on commercial land; the
        // rest blocks of four and five (a block of townhouses under one outline, a market, a car park); a school's
        // or a hospital's a block.
        let large = est(1800.0, 1.5, 0.45, homes);
        let towers: Vec<_> = large.iter().filter(|b| b.0 == Kind::Tower).collect();
        assert_eq!(towers.len(), 12, "{large:?}");
        assert!(towers.iter().all(|(_, s)| (12..=25).contains(s)));
        assert!(towers.iter().any(|b| b.1 <= 14) && towers.iter().any(|b| b.1 >= 23));
        assert!(
            large
                .iter()
                .filter(|b| b.0 != Kind::Tower)
                .all(|(k, s)| *k == Kind::Block && (4..=5).contains(s))
        );
        assert!(large.iter().any(|b| b.1 == 4) && large.iter().any(|b| b.1 == 5));
        let shops = est(2500.0, 1.5, 0.5, Usage::Commercial);
        assert_eq!(shops.iter().filter(|b| b.0 == Kind::Office).count(), 12);
        assert!(
            shops
                .iter()
                .all(|(k, s)| (*k == Kind::Office && *s >= 12) || (*k == Kind::Block && *s <= 5))
        );
        assert!(
            est(2000.0, 1.3, 0.6, Usage::Civic)
                .iter()
                .all(|(k, s)| *k == Kind::Block && (4..=5).contains(s))
        );
        // A shophouse walk-up on commercial land, not an office; a market's or an estate's whole block under one
        // outline five to eight, not a tower.
        assert!(
            est(500.0, 1.3, 0.45, Usage::Commercial)
                .iter()
                .all(|(k, _)| *k == Kind::Block)
        );
        assert!(
            est(6000.0, 1.5, 0.5, homes)
                .iter()
                .all(|(k, s)| *k == Kind::Block && (5..=8).contains(s))
        );
        // A long row's outline its townhouses, an industrial estate's sheds, a village's factory.
        assert!(
            est(1500.0, 5.0, 0.45, homes)
                .iter()
                .all(|(k, s)| *k == Kind::Terrace && (4..=5).contains(s))
        );
        assert!(
            est(1500.0, 1.5, 0.45, Usage::Industrial)
                .iter()
                .all(|(k, _)| *k != Kind::Tower)
        );
        assert!(
            est(1600.0, 1.5, 0.08, Usage::Unknown)
                .iter()
                .all(|(k, s)| *k == Kind::Industrial && *s == 2)
        );
        // No detached house anywhere Taiwan's estimate reaches; East Asia's as it was.
        for density in [0.05, 0.2, 0.4, 0.7] {
            assert!(
                est(150.0, 1.4, density, homes)
                    .iter()
                    .all(|(k, _)| *k != Kind::House)
            );
        }
        assert_eq!(
            estimate(150.0, 1.4, 0.45, 0.5, Region::EastAsia, homes),
            (Kind::House, 2)
        );
    }

    #[test]
    fn small_houses_follow_the_region() {
        let homes = Usage::Residential;
        let count = |area: f32, density: f32, region: Region, want: (Kind, u8)| {
            (0..20)
                .filter(|k| estimate(area, 1.6, density, *k as f32 / 20.0, region, homes) == want)
                .count()
        };
        // Terraces wherever a British, Irish or Dutch town is built over; two-storey houses about it (cottages of one
        // in Scotland and Ireland now and then); Florida's single-storey houses.
        for region in [Region::British, Region::Celtic, Region::NorthSea] {
            let terraces = (0..20)
                .filter(|k| {
                    estimate(60.0, 2.0, 0.45, *k as f32 / 20.0, region, homes).0 == Kind::Terrace
                })
                .count();
            assert_eq!(terraces, 20, "{region:?}");
        }
        assert!(count(90.0, 0.15, Region::British, (Kind::House, 2)) >= 16);
        assert!(count(90.0, 0.15, Region::Celtic, (Kind::House, 1)) >= 6);
        assert!(count(180.0, 0.2, Region::Florida, (Kind::House, 1)) >= 14);
        // Malta's townhouses, Athens' and the delta's narrow blocks where the ground is built over.
        let kinds = |area: f32, density: f32, region: Region| {
            (0..20)
                .map(|k| estimate(area, 1.6, density, k as f32 / 20.0, region, homes))
                .collect::<Vec<_>>()
        };
        assert!(
            kinds(90.0, 0.5, Region::Malta)
                .iter()
                .all(|(k, s)| *k == Kind::Terrace && (2..=3).contains(s))
        );
        assert!(
            kinds(150.0, 0.55, Region::Aegean)
                .iter()
                .all(|(k, s)| *k == Kind::Block && (4..=6).contains(s))
        );
        assert!(
            kinds(120.0, 0.35, Region::Nile)
                .iter()
                .all(|(k, s)| *k == Kind::Block && (3..=5).contains(s))
        );
        assert!(
            kinds(600.0, 0.4, Region::Aegean)
                .iter()
                .all(|(k, s)| *k == Kind::Block && (4..=7).contains(s))
        );
        // Hawaii's houses of one storey and two, Guam's and the atolls' of one (two now and then).
        assert!(count(150.0, 0.35, Region::Hawaii, (Kind::House, 1)) >= 11);
        assert!(count(150.0, 0.35, Region::Hawaii, (Kind::House, 2)) >= 6);
        for region in [Region::Micronesia, Region::Atoll] {
            assert!(
                count(100.0, 0.5, region, (Kind::House, 1)) >= 16,
                "{region:?}"
            );
        }
        assert!(
            kinds(300.0, 0.5, Region::Micronesia)
                .iter()
                .all(|(k, s)| *k == Kind::Block && (2..=3).contains(s))
        );
        // No random tower in Micronesia or on an atoll, however dense the ground.
        let flat = |_: [f32; 2]| 10.0f32;
        for region in [Region::Micronesia, Region::Atoll] {
            for k in 0..200 {
                let b = plan_building(
                    &rect(0.0, 0.0, 30.0, 25.0),
                    [k, 7],
                    0.6,
                    region,
                    Period::Today,
                    &on(homes),
                    &flat,
                )
                .unwrap();
                assert_ne!(b.kind, Kind::Tower, "{region:?}");
                assert!(b.height < 15.0, "{region:?} {}", b.height);
            }
        }
    }

    #[test]
    fn a_terraced_house_has_its_ridge_along_the_street() {
        let flat = |_: [f32; 2]| 10.0f32;
        let homes = on(Usage::Residential);
        // A row of 5 m wide, 10 m deep houses in a built-over British street: each a terrace whose ridge runs across its
        // width (the ring's first side), so the row's roofs run on as one; gabled, never hipped, no taller than 4.2 m.
        for k in 0..12 {
            let house = rect(5.0 * k as f32, 0.0, 5.0, 10.0);
            let b = plan_building(
                &house,
                [k, 0],
                0.5,
                Region::British,
                Period::Today,
                &homes,
                &flat,
            )
            .unwrap();
            assert_eq!(b.kind, Kind::Terrace);
            let Roof::Gabled { rise } = b.roof else {
                panic!("{:?}", b.roof);
            };
            assert!((2.5..=4.2).contains(&rise), "{rise}");
            assert!((geometry::distance(b.ring[0], b.ring[1]) - 5.0).abs() < 0.01);
        }
        // A detached house keeps its ridge along its longer side.
        let house = plan_building(
            &rect(0.0, 0.0, 6.0, 10.0),
            [0, 0],
            0.1,
            Region::British,
            Period::Today,
            &homes,
            &flat,
        )
        .unwrap();
        assert_eq!(house.kind, Kind::House);
        assert!((geometry::distance(house.ring[0], house.ring[1]) - 10.0).abs() < 0.01);
    }

    #[test]
    fn density_is_the_share_built_over() {
        // A 100 m cell holding 25 squares of 10 m: 25% built, its neighbours' average less.
        let mut fps = Vec::new();
        for j in 0..5 {
            for i in 0..5 {
                fps.push(square(
                    505.0 + 20.0 * i as f32,
                    505.0 + 20.0 * j as f32,
                    10.0,
                ));
            }
        }
        let d = DensityGrid::new(&fps);
        let here = d.at([550.0, 550.0]);
        assert!((here - 0.25 * 2.0 / 10.0).abs() < 1e-3, "{here}");
        assert_eq!(d.at([50.0, 50.0]), 0.0);
    }

    #[test]
    fn chunks_tile_the_theatre() {
        assert_eq!(chunk_of([0.0, 0.0]), [0, 0]);
        assert_eq!(chunk_of([-0.1, 999.9]), [-1, 0]);
        assert_eq!(chunk_corner([-3, 2]), [-3000.0, 2000.0]);
        assert_eq!(cube_entry([0, 0, 0]), CUBE_FIRST);
        assert_eq!(cube_entry([255, 255, 255]), CUBE_FIRST + 215);
    }
}
