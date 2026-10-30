//! What a chunk's buildings know of the map about them, in chunk-local metres: OpenStreetMap's land use, points of
//! interest and building outlines (their heights and colours), the water and the piers, and the few landmarks the host
//! knows by name (native/docs/terrain/buildings.md, "Types"). The host (`naval-client`'s `theatre::buildings`) fills
//! [`MapTags`] from the OpenMapTiles layers; this module reads a footprint's setting from them ([`MapTags::setting`]:
//! the land use it stands in, the point of interest on it, the sea beside it, a pier under it), merges OSM's outlines
//! with the footprints ([`merge_outlines`]) and finds the quays cranes stand on ([`MapTags::quays`]).
//!
//! OpenMapTiles carries no building type: a footprint's type comes from the land use about it and the points of
//! interest on it, and from its own shape ([`super::plan_building`]).

use super::{CHUNK_M, Footprint, Part, geometry};

/// What the land about a building is used for, from OpenMapTiles' `landuse` classes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Usage {
    #[default]
    Unknown,
    Residential,
    /// Commercial and retail land.
    Commercial,
    /// Industrial land, railway yards and garages; a harbour's land is industrial land at the water.
    Industrial,
    /// Schools, universities, hospitals, libraries.
    Civic,
    Military,
    /// Land nothing stands on but a stand or a shed: cemeteries, pitches, playgrounds, parks, zoos.
    Open,
}

impl Usage {
    /// The usage of an OpenMapTiles `landuse` class, none for a class that says nothing of buildings (the `suburb`,
    /// `quarter` and `neighbourhood` places, `dam`).
    pub fn of_landuse(class: &str) -> Option<Self> {
        Some(match class {
            "residential" => Usage::Residential,
            "commercial" | "retail" => Usage::Commercial,
            "industrial" | "railway" | "garages" | "port" | "harbour" => Usage::Industrial,
            "military" => Usage::Military,
            "school" | "university" | "college" | "kindergarten" | "hospital" | "library"
            | "education" | "public_facility" | "bus_station" => Usage::Civic,
            "cemetery" | "pitch" | "playground" | "track" | "stadium" | "theme_park" | "zoo"
            | "recreation_ground" | "quarry" => Usage::Open,
            _ => return None,
        })
    }

    /// How specific a usage is: where zones overlap (a school within a residential district) the more specific wins.
    fn rank(self) -> u8 {
        match self {
            Usage::Unknown => 0,
            Usage::Residential => 1,
            Usage::Commercial => 2,
            Usage::Civic | Usage::Industrial | Usage::Military => 3,
            Usage::Open => 4,
        }
    }
}

/// A place of worship's religion (OpenMapTiles' `poi` `subclass`), for its form: a spire, a dome, minarets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Religion {
    Christian,
    Muslim,
    Jewish,
    Buddhist,
    Hindu,
    Other,
}

impl Religion {
    pub fn of(subclass: Option<&str>) -> Self {
        match subclass {
            Some("christian") | None => Religion::Christian,
            Some("muslim") => Religion::Muslim,
            Some("jewish") => Religion::Jewish,
            Some("buddhist") | Some("shinto") | Some("taoist") => Religion::Buddhist,
            Some("hindu") | Some("jain") | Some("sikh") => Religion::Hindu,
            Some(_) => Religion::Other,
        }
    }
}

/// A point of interest that types the building it stands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoiKind {
    Worship(Religion),
    /// OpenMapTiles has no lighthouses: one known by its name (`is_lighthouse_name`).
    Lighthouse,
    /// A harbour or marina (`harbor`), a ferry terminal.
    Harbour,
    Fuel,
}

impl PoiKind {
    /// The kind of an OpenMapTiles point of interest, none for one that does not type its building.
    pub fn of(class: &str, subclass: Option<&str>, name: Option<&str>) -> Option<Self> {
        match class {
            "place_of_worship" => Some(PoiKind::Worship(Religion::of(subclass))),
            "harbor" | "ferry_terminal" => Some(PoiKind::Harbour),
            "fuel" if subclass != Some("charging_station") => Some(PoiKind::Fuel),
            "attraction" | "museum" | "monument" | "castle" | "information" | ""
                if name.is_some_and(is_lighthouse_name) =>
            {
                Some(PoiKind::Lighthouse)
            }
            _ => None,
        }
    }
}

/// Whether a name is a lighthouse's ("Point Sur Lighthouse", "Phare de Cordouan", "Gróttuviti", "Lindesnes fyr",
/// "Ahırkapı Feneri"): the word for a lighthouse at the head of the name, not inside another (a "Lighthouse Bank", a
/// "Lighthouse Field State Beach", which the point's class leaves out anyway).
pub fn is_lighthouse_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    let ends = |w: &str| n.ends_with(w);
    let starts = |w: &str| n.starts_with(w);
    ends("lighthouse")
        || ends("light station")
        || ends("light house")
        || starts("phare ")
        || starts("faro ")
        || starts("farol ")
        || starts("leuchtturm")
        || ends("leuchtturm")
        || ends("viti")
        || ends(" fyr")
        || ends("fyr")
        || ends("vuurtoren")
        || ends("majakka")
        || ends("feneri")
        || starts("latarnia")
        || n.contains("маяк")
        || ends("灯台")
        || ends("灯塔")
}

/// A land-use polygon.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub usage: Usage,
    pub exterior: Vec<[f32; 2]>,
    pub holes: Vec<Vec<[f32; 2]>>,
    /// Min x, min z, max x, max z.
    pub bounds: [f32; 4],
    /// Railway land (counted as industrial): an old town's mark in the period ([`super::thinning::lift`]).
    pub railway: bool,
}

impl Zone {
    pub fn new(usage: Usage, exterior: Vec<[f32; 2]>, holes: Vec<Vec<[f32; 2]>>) -> Self {
        let bounds = bounds_of(&exterior);
        Self {
            usage,
            exterior,
            holes,
            bounds,
            railway: false,
        }
    }

    pub fn holds(&self, p: [f32; 2]) -> bool {
        inside(&self.bounds, p) && geometry::contains_with_holes(&self.exterior, &self.holes, p)
    }
}

/// A point of interest that types a building.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Poi {
    pub at: [f32; 2],
    pub kind: PoiKind,
}

/// A building's outline as OpenStreetMap maps it: its height (m, from `height` or `building:levels`; none when the
/// schema only gave its default), where a part starts above the ground, its colour, and whether another part draws it
/// (`hide_3d`).
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub ring: Vec<[f32; 2]>,
    pub height: Option<f32>,
    pub min_height: f32,
    pub colour: Option<[u8; 3]>,
    pub hide_3d: bool,
}

/// Water: the sea (OpenMapTiles' `ocean`), a dock, a river's banks.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterArea {
    pub exterior: Vec<[f32; 2]>,
    pub holes: Vec<Vec<[f32; 2]>>,
    pub bounds: [f32; 4],
}

impl WaterArea {
    pub fn new(exterior: Vec<[f32; 2]>, holes: Vec<Vec<[f32; 2]>>) -> Self {
        let bounds = bounds_of(&exterior);
        Self {
            exterior,
            holes,
            bounds,
        }
    }

    fn holds(&self, p: [f32; 2]) -> bool {
        inside(&self.bounds, p) && geometry::contains_with_holes(&self.exterior, &self.holes, p)
    }
}

/// A pier's deck: its ring, and whether it came from a line (a jetty or a pontoon, drawn narrow and low).
#[derive(Clone, Debug, PartialEq)]
pub struct Pier {
    pub ring: Vec<[f32; 2]>,
    pub narrow: bool,
    pub bounds: [f32; 4],
}

impl Pier {
    /// A pier mapped as an area.
    pub fn area(ring: Vec<[f32; 2]>) -> Option<Self> {
        let ring = geometry::simplify(&ring, 0.3)?;
        let bounds = bounds_of(&ring);
        Some(Self {
            ring,
            narrow: false,
            bounds,
        })
    }

    /// A pier mapped as a line: a strip [`PIER_LINE_M`] wide along it.
    pub fn line(points: &[[f32; 2]]) -> Option<Self> {
        let ring = geometry::buffer_line(points, PIER_LINE_M * 0.5)?;
        let bounds = bounds_of(&ring);
        Some(Self {
            ring,
            narrow: true,
            bounds,
        })
    }

    /// The deck's height above the sea (m): a wharf's 2.4 m, a jetty's or pontoon's 1.1 m.
    pub fn deck_m(&self) -> f32 {
        if self.narrow { 1.1 } else { 2.4 }
    }

    fn holds(&self, p: [f32; 2]) -> bool {
        inside(&self.bounds, p) && geometry::contains(&self.ring, p)
    }
}

/// A pier line's deck width (m).
pub const PIER_LINE_M: f32 = 4.0;

/// A landmark the host knows by name and place (`naval-client`'s `theatre::buildings::LANDMARKS`): its tower stands
/// whether or not a footprint does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landmark {
    pub at: [f32; 2],
    pub kind: LandmarkKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LandmarkKind {
    /// A lighthouse `height` m to its lantern's top; `bands` red or black bands up its tower (0 plain white), the
    /// lantern's cap black or red; `keeper` when the tower rises from its keeper's house (Point Pinos).
    Lighthouse {
        height: f32,
        bands: u8,
        red: bool,
        keeper: bool,
    },
    /// A free-standing stack `height` m tall, `radius` m at its foot.
    Chimney { height: f32, radius: f32 },
}

/// A stretch of road or railway: from `a` to `b`, `half` its half width (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Road {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub half: f32,
}

/// Everything a chunk's buildings read of the map, chunk-local. Empty: the footprints alone, typed by their shape and
/// size as before the map was read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapTags {
    pub zones: Vec<Zone>,
    pub pois: Vec<Poi>,
    pub outlines: Vec<Outline>,
    /// OSM's outlines whose middles lie in the chunks about this one, within the map's reach of it: they stand in their
    /// own chunks, and are read here only for the building a part of this chunk's belongs to ([`merge_outlines`]).
    pub outlines_about: Vec<Outline>,
    pub water: Vec<WaterArea>,
    pub piers: Vec<Pier>,
    pub landmarks: Vec<Landmark>,
    /// The roads and railways (no driveways or tracks), which the infill's lots keep off.
    pub roads: Vec<Road>,
    /// Whether a working port is near (a ferry terminal within a tile or so): only its quays get cranes. OpenMapTiles
    /// maps no port land and no cranes, and a marina's or a power station's waterfront is industrial land at the water
    /// too.
    pub port: bool,
    /// Areas the host draws itself (a landmark of its own; rings, any orientation): nothing whose middle lies in one
    /// stands (a footprint, a part, a landmark, a lighthouse, a crane, a pier's deck), nor an infill lot reaching into
    /// one ([`MapTags::cleared_at`]).
    pub cleared: Vec<Vec<[f32; 2]>>,
}

/// What a footprint stands in: its land's usage, the point of interest on it, whether the sea is near, and the pier deck
/// under it (its height above the sea) when it stands on one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Setting {
    pub usage: Usage,
    pub poi: Option<PoiKind>,
    pub near_sea: bool,
    pub pier: Option<f32>,
    /// In a past period, the tallest its place allowed (m, `thinning::Thinning::cap_for`): a measured building taller
    /// keeps the kind its height gives and stands at this. None today.
    pub cap: Option<f32>,
}

fn bounds_of(ring: &[[f32; 2]]) -> [f32; 4] {
    ring.iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        })
}

fn inside(b: &[f32; 4], p: [f32; 2]) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

/// A stretch of quay: from `a` to `b` along the water's edge (chunk-local, on land), `out` the unit direction to the
/// water, and the area of the industrial land behind it (m²).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quay {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub out: [f32; 2],
    pub land_m2: f32,
}

impl Quay {
    pub fn length(&self) -> f32 {
        geometry::distance(self.a, self.b)
    }
}

impl MapTags {
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
            && self.pois.is_empty()
            && self.outlines.is_empty()
            && self.water.is_empty()
            && self.piers.is_empty()
            && self.landmarks.is_empty()
            && self.roads.is_empty()
    }

    /// Whether chunk-local point `p` lies in an area the host draws itself ([`MapTags::cleared`]).
    pub fn cleared_at(&self, p: [f32; 2]) -> bool {
        self.cleared.iter().any(|ring| {
            ring.len() >= 3 && inside(&bounds_of(ring), p) && geometry::contains(ring, p)
        })
    }

    /// Whether chunk-local `ring` reaches into an area the host draws itself at all ([`MapTags::cleared`]): their
    /// edges cross, or one holds the other's point (an infill lot's wall through a narrow area, or a small area within
    /// the lot).
    pub fn cleared_over(&self, ring: &[[f32; 2]]) -> bool {
        if self.cleared.is_empty() || ring.len() < 3 {
            return false;
        }
        let b = bounds_of(ring);
        self.cleared.iter().any(|area| {
            let a = bounds_of(area);
            a[0] <= b[2]
                && a[2] >= b[0]
                && a[1] <= b[3]
                && a[3] >= b[1]
                && geometry::overlaps(area, ring)
        })
    }

    /// Whether a road or railway crosses a footprint: its strip reaches into the circle about the footprint's middle
    /// that the footprint holds (an infill lot the map's street runs through).
    pub fn on_road(&self, ring: &[[f32; 2]]) -> bool {
        if ring.len() < 3 {
            return false;
        }
        let c = geometry::centroid(ring);
        let inner = geometry::ring_distance(ring, c) * 0.9;
        let reach = inner + 8.0;
        self.roads.iter().any(|r| {
            let lo = [r.a[0].min(r.b[0]) - reach, r.a[1].min(r.b[1]) - reach];
            let hi = [r.a[0].max(r.b[0]) + reach, r.a[1].max(r.b[1]) + reach];
            c[0] >= lo[0]
                && c[0] <= hi[0]
                && c[1] >= lo[1]
                && c[1] <= hi[1]
                && geometry::segment_distance(c, r.a, r.b) < r.half + inner
        })
    }

    /// The usage of the land at `p`: the most specific zone holding it.
    pub fn usage_at(&self, p: [f32; 2]) -> Usage {
        self.zones
            .iter()
            .filter(|z| z.holds(p))
            .map(|z| z.usage)
            .max_by_key(|u| u.rank())
            .unwrap_or_default()
    }

    /// Whether `p` lies on the water the map knows (the sea, a dock, a river), or with no water mapped, where the
    /// ground is under the sea.
    pub fn is_water(&self, p: [f32; 2], ground: &dyn Fn([f32; 2]) -> f32) -> bool {
        if self.water.is_empty() {
            ground(p) < -0.5
        } else {
            self.water.iter().any(|w| w.holds(p))
        }
    }

    /// Whether `p` lies on the water the map draws (the sea, a dock, a river), whatever the ground.
    pub fn on_mapped_water(&self, p: [f32; 2]) -> bool {
        self.water.iter().any(|w| w.holds(p))
    }

    /// The pier under `p`, if any.
    pub fn pier_at(&self, p: [f32; 2]) -> Option<&Pier> {
        self.piers.iter().find(|q| q.holds(p))
    }

    /// The setting of a footprint whose ring is `ring` (chunk-local): the point of interest within it (or within 6 m
    /// of it), the zone its middle stands in, the pier under its middle, and for a round footprint (a tank's) whether
    /// the sea lies within about 150 m (the ground under the sea there).
    pub fn setting(&self, ring: &[[f32; 2]], ground: &dyn Fn([f32; 2]) -> f32) -> Setting {
        let c = geometry::centroid(ring);
        let b = bounds_of(ring);
        let near = |p: [f32; 2], m: f32| {
            p[0] >= b[0] - m && p[0] <= b[2] + m && p[1] >= b[1] - m && p[1] <= b[3] + m
        };
        let poi = self
            .pois
            .iter()
            .filter(|q| near(q.at, 6.0))
            .find(|q| geometry::contains(ring, q.at) || geometry::ring_distance(ring, q.at) < 6.0)
            .map(|q| q.kind);
        let mut near_sea = false;
        let round = ring.len() >= 6 && geometry::roundness(ring) >= 0.8;
        'rings: for r in [70.0f32, 150.0] {
            if !round {
                break;
            }
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                if ground([c[0] + r * a.cos(), c[1] + r * a.sin()]) < -0.5 {
                    near_sea = true;
                    break 'rings;
                }
            }
        }
        Setting {
            usage: self.usage_at(c),
            poi,
            near_sea,
            pier: self.pier_at(c).map(Pier::deck_m),
            cap: None,
        }
    }

    /// The quays: stretches of industrial land's edge with the water beyond (a point every 8 m whose 18 m outward
    /// lies on the water and whose 10 m inward on land), joined where they run on, each at least `min_m` long.
    pub fn quays(&self, ground: &dyn Fn([f32; 2]) -> f32, min_m: f32) -> Vec<Quay> {
        let mut out = Vec::new();
        let step = 8.0f32;
        for zone in self.zones.iter().filter(|z| z.usage == Usage::Industrial) {
            let ring = &zone.exterior;
            let n = ring.len();
            if n < 3 {
                continue;
            }
            let land_m2 = geometry::area(ring);
            let flip = if geometry::signed_area2(ring) < 0.0 {
                -1.0
            } else {
                1.0
            };
            let mut run: Option<Quay> = None;
            let close = |run: &mut Option<Quay>, out: &mut Vec<Quay>| {
                if let Some(q) = run.take()
                    && q.length() >= min_m
                {
                    out.push(q);
                }
            };
            for i in 0..n {
                let (a, c) = (ring[i], ring[(i + 1) % n]);
                let len = geometry::distance(a, c);
                if len < 1.0 {
                    continue;
                }
                let o = [flip * (c[1] - a[1]) / len, flip * -(c[0] - a[0]) / len];
                let steps = (len / step).floor().max(1.0) as usize;
                for k in 0..steps {
                    let t = (k as f32 + 0.5) / steps as f32;
                    let p = [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t];
                    // Only within the chunk (a zone cut at a tile's edge has a straight side there).
                    if !(0.0..CHUNK_M as f32).contains(&p[0])
                        || !(0.0..CHUNK_M as f32).contains(&p[1])
                    {
                        close(&mut run, &mut out);
                        continue;
                    }
                    let wet = self.is_water([p[0] + o[0] * 18.0, p[1] + o[1] * 18.0], ground);
                    let back = [p[0] - o[0] * 10.0, p[1] - o[1] * 10.0];
                    let dry = !self.is_water(back, ground) && ground(back) > 0.3;
                    if wet && dry {
                        match &mut run {
                            // Running on along much the same heading.
                            Some(q)
                                if q.out[0] * o[0] + q.out[1] * o[1] > 0.8
                                    && geometry::distance(q.b, p) < step * 2.0 =>
                            {
                                q.b = p;
                            }
                            _ => {
                                close(&mut run, &mut out);
                                run = Some(Quay {
                                    a: p,
                                    b: p,
                                    out: o,
                                    land_m2,
                                });
                            }
                        }
                    } else {
                        close(&mut run, &mut out);
                    }
                }
            }
            close(&mut run, &mut out);
        }
        out
    }
}

/// The most a footprint source's height is believed (m): Microsoft's model stops at about 35 m, so a height there is a
/// cap, and OpenStreetMap's (when it has one) is taken instead.
pub const CAPPED_M: f32 = 34.0;

/// The least `min_height` that starts a part above the ground (m): below it, it stands on the ground.
pub const PART_BASE_M: f32 = 0.5;

/// Merges OpenStreetMap's outlines with the footprints (both chunk-local): a footprint whose middle an outline holds
/// takes that outline's height where it has none or a capped one ([`CAPPED_M`]), and its colour; an outline that holds
/// no footprint's middle, and whose own middle no footprint holds, is added as a footprint of its own (a building
/// Microsoft's data misses), unless its parts draw it (`hide_3d`).
///
/// A building mapped as parts (`building:part`s, one on another: a tower's shaft, its crown, its spire) stands as its
/// parts, each a footprint of its own from its base (`min_height`) to its height ([`Part`]), all read at their
/// building's middle; the footprint whose middle lies in such a building (an outline drawn by parts it holds, or a part
/// raised over the ground with a part on the ground there too) is left out, the parts standing in its place (a part on
/// the ground with no height of its own takes the footprint's). The outlines `about` the chunk (whose middles lie in
/// its neighbours, [`MapTags::outlines_about`]) are read for the building a part belongs to, and stand nowhere here:
/// a tower whose parts lie across a chunk's edge keeps one middle, and its parts meet.
pub fn merge_outlines(footprints: &mut Vec<Footprint>, outlines: &[Outline], about: &[Outline]) {
    if outlines.is_empty() && about.is_empty() {
        return;
    }
    // The chunk's own outlines first, then those about it.
    let all: Vec<&Outline> = outlines.iter().chain(about).collect();
    let own = |o: usize| o < outlines.len();
    let grid = CellGrid::new();
    // The outlines by the cells their boxes touch, and the footprints likewise: every point query reads one cell.
    let obounds: Vec<[f32; 4]> = all.iter().map(|o| bounds_of(&o.ring)).collect();
    let mut ocells: Vec<Vec<u32>> = vec![Vec::new(); grid.len()];
    for (o, b) in obounds.iter().enumerate() {
        if all[o].ring.len() >= 3 {
            grid.insert(&mut ocells, b, o as u32);
        }
    }
    let fbounds: Vec<[f32; 4]> = footprints.iter().map(|f| bounds_of(&f.ring)).collect();
    let mut fcells: Vec<Vec<u32>> = vec![Vec::new(); grid.len()];
    for (k, b) in fbounds.iter().enumerate() {
        if footprints[k].ring.len() >= 3 {
            grid.insert(&mut fcells, b, k as u32);
        }
    }
    // The outlines holding point `p`, and the footprints.
    let (grid, ocells, obounds, fcells, fbounds, all) =
        (&grid, &ocells, &obounds, &fcells, &fbounds, &all);
    let outlines_at = move |p: [f32; 2]| {
        ocells[grid.cell(p)]
            .iter()
            .map(|&o| o as usize)
            .filter(move |&o| inside(&obounds[o], p) && geometry::contains(&all[o].ring, p))
    };
    let held: &[Footprint] = footprints;
    let footprints_at = move |p: [f32; 2]| {
        fcells[grid.cell(p)]
            .iter()
            .map(|&k| k as usize)
            .filter(move |&k| inside(&fbounds[k], p) && geometry::contains(&held[k].ring, p))
    };
    let middles: Vec<[f32; 2]> = all
        .iter()
        .map(|o| {
            if o.ring.len() >= 3 {
                geometry::centroid(&o.ring)
            } else {
                [f32::NAN; 2]
            }
        })
        .collect();
    // What each outline is: drawn by its parts (`hide_3d`), a part raised over the ground (a base and a height over
    // it), or standing on the ground.
    let raised: Vec<bool> = all
        .iter()
        .map(|o| {
            !o.hide_3d
                && o.min_height >= PART_BASE_M
                && o.height.is_some_and(|h| h >= o.min_height + 1.0)
        })
        .collect();
    let on_ground = |o: usize| !all[o].hide_3d && all[o].min_height < PART_BASE_M;
    let real = |o: usize| all[o].ring.len() >= 3;
    // An outline drawn by its parts that holds one of them that stands (its parts are in the data, not only beyond the
    // chunk's reach; a part with no height over its base stands nowhere, and leaves the footprint under it standing); a
    // footprint holding a raised part's middle or a grounded outline's; a grounded outline holding a raised part's.
    let mut has_parts = vec![false; all.len()];
    let mut holds_raised = vec![false; footprints.len()];
    let mut holds_ground = vec![false; footprints.len()];
    let mut under_raised = vec![false; all.len()];
    for o in (0..all.len()).filter(|&o| real(o) && (raised[o] || on_ground(o))) {
        for h in outlines_at(middles[o]) {
            if all[h].hide_3d {
                has_parts[h] = true;
            } else if raised[o] && on_ground(h) {
                under_raised[h] = true;
            }
        }
        for k in footprints_at(middles[o]) {
            if raised[o] {
                holds_raised[k] = true;
            } else if on_ground(o) {
                holds_ground[k] = true;
            }
        }
    }
    // The footprints a building's parts draw, left out.
    let drawn: Vec<bool> = footprints
        .iter()
        .enumerate()
        .map(|(k, f)| {
            if f.ring.len() < 3 {
                return false;
            }
            let (mut hidden, mut in_raised, mut in_ground) = (false, false, false);
            for o in outlines_at(geometry::centroid(&f.ring)) {
                hidden |= has_parts[o];
                in_raised |= raised[o];
                in_ground |= on_ground(o);
            }
            hidden || ((in_raised || holds_raised[k]) && (in_ground || holds_ground[k]))
        })
        .collect();
    let mut matched = vec![false; all.len()];
    // Each footprint's height and colour from the outlines over it (set once the queries are done).
    let mut lent: Vec<(usize, Option<f32>, Option<[u8; 3]>)> = Vec::new();
    for (k, f) in footprints.iter().enumerate() {
        if f.ring.len() < 3 || drawn[k] {
            continue;
        }
        let m = geometry::centroid(&f.ring);
        // The largest outline holding the footprint's middle is its building (a tower's part lies within its
        // church's outline); the tallest gives its height.
        let mut best: Option<(f32, usize)> = None;
        let mut tallest: Option<f32> = None;
        for o in outlines_at(m).filter(|&o| own(o)) {
            matched[o] = true;
            let area = geometry::area(&all[o].ring);
            if best.is_none_or(|(a, _)| area > a) {
                best = Some((area, o));
            }
            if let Some(h) = all[o].height {
                tallest = Some(tallest.map_or(h, |t: f32| t.max(h)));
            }
        }
        let Some((_, o)) = best else { continue };
        let capped = f.height.is_none_or(|h| !(2.5..CAPPED_M).contains(&h));
        let height = match tallest.filter(|h| f.height.is_none_or(|old| *h > old)) {
            Some(h) if capped => Some(h),
            _ => f.height,
        };
        lent.push((k, height, f.colour.or(all[o].colour)));
    }
    // A part's building's middle: the largest outline drawn by its parts, footprint left out for them or grounded
    // outline over point `p` (itself, for a grounded one); `p` where none is.
    let middle_of = |p: [f32; 2]| -> [f32; 2] {
        let mut best: Option<(f32, [f32; 2])> = None;
        let mut consider = |ring: &[[f32; 2]]| {
            let area = geometry::area(ring);
            if best.is_none_or(|(a, _)| area > a) {
                best = Some((area, geometry::centroid(ring)));
            }
        };
        for o in outlines_at(p).filter(|&o| has_parts[o] || on_ground(o)) {
            consider(&all[o].ring);
        }
        for k in footprints_at(p).filter(|&k| drawn[k]) {
            consider(&footprints[k].ring);
        }
        best.map_or(p, |(_, middle)| middle)
    };
    let mut added = Vec::new();
    for (o, outline) in outlines.iter().enumerate() {
        if matched[o] || !real(o) || outline.hide_3d {
            continue;
        }
        let m = middles[o];
        let footprint = |height: Option<f32>, part: Option<Part>| Footprint {
            ring: outline.ring.clone(),
            height,
            kind: None,
            colour: outline.colour,
            part,
        };
        if raised[o] {
            added.push(footprint(
                outline.height,
                Some(Part {
                    base: outline.min_height,
                    middle: middle_of(m),
                }),
            ));
            continue;
        }
        // A part over the ground with no height to stand to.
        if outline.min_height >= PART_BASE_M {
            continue;
        }
        if footprints_at(m).any(|k| !drawn[k]) {
            continue;
        }
        // The footprint left out for it (the largest), whose height it takes where it has none.
        let left_out = footprints_at(m).filter(|&k| drawn[k]).max_by(|&a, &b| {
            geometry::area(&footprints[a].ring).total_cmp(&geometry::area(&footprints[b].ring))
        });
        let part_of = outlines_at(m).any(|h| has_parts[h]) || left_out.is_some() || under_raised[o];
        let height = outline
            .height
            .or_else(|| left_out.and_then(|k| footprints[k].height));
        added.push(footprint(
            height,
            part_of.then(|| Part {
                base: 0.0,
                middle: middle_of(m),
            }),
        ));
    }
    for (k, height, colour) in lent {
        footprints[k].height = height;
        footprints[k].colour = colour;
    }
    footprints.retain({
        let mut k = 0;
        move |_| {
            k += 1;
            !drawn[k - 1]
        }
    });
    footprints.extend(added);
}

/// 50 m cells over a chunk and a cell's ring about it (a point outside falls in the nearest edge cell).
struct CellGrid {
    cells: i32,
}

impl CellGrid {
    const CELL: f32 = 50.0;

    fn new() -> Self {
        Self {
            cells: (CHUNK_M as f32 / Self::CELL) as i32 + 2,
        }
    }

    fn len(&self) -> usize {
        (self.cells * self.cells) as usize
    }

    fn ij(&self, p: [f32; 2]) -> (i32, i32) {
        (
            ((p[0] / Self::CELL).floor() as i32 + 1).clamp(0, self.cells - 1),
            ((p[1] / Self::CELL).floor() as i32 + 1).clamp(0, self.cells - 1),
        )
    }

    fn cell(&self, p: [f32; 2]) -> usize {
        let (i, j) = self.ij(p);
        (j * self.cells + i) as usize
    }

    /// Files item `k` under every cell its box touches.
    fn insert(&self, cells: &mut [Vec<u32>], b: &[f32; 4], k: u32) {
        let (i0, j0) = self.ij([b[0], b[1]]);
        let (i1, j1) = self.ij([b[2], b[3]]);
        for j in j0..=j1 {
            for i in i0..=i1 {
                cells[(j * self.cells + i) as usize].push(k);
            }
        }
    }
}

/// An sRGB colour from a CSS colour as OpenStreetMap's `building:colour` gives it: `#rgb`, `#rrggbb` or one of CSS's
/// named colours (case and spaces aside); none for anything else.
pub fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let t: String = text
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some(hex) = t.strip_prefix('#') {
        let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        let b = hex.as_bytes();
        return match b.len() {
            3 => Some([digit(b[0])? * 17, digit(b[1])? * 17, digit(b[2])? * 17]),
            6 => Some([
                digit(b[0])? * 16 + digit(b[1])?,
                digit(b[2])? * 16 + digit(b[3])?,
                digit(b[4])? * 16 + digit(b[5])?,
            ]),
            _ => None,
        };
    }
    let name = t.replace("grey", "gray");
    CSS_COLOURS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, rgb)| [(rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8])
}

/// CSS's named colours (CSS Color Module Level 4), "grey" spelt "gray".
const CSS_COLOURS: [(&str, u32); 141] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, z: f32, side: f32) -> Vec<[f32; 2]> {
        vec![[x, z], [x + side, z], [x + side, z + side], [x, z + side]]
    }

    #[test]
    fn colours_parse_as_css_gives_them() {
        assert_eq!(parse_colour("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_colour("#B22222"), Some([178, 34, 34]));
        assert_eq!(parse_colour("firebrick"), parse_colour("#b22222"));
        assert_eq!(parse_colour("Dark Grey"), Some([169, 169, 169]));
        assert_eq!(parse_colour("light_grey"), Some([211, 211, 211]));
        assert_eq!(parse_colour("paleturquoise"), Some([175, 238, 238]));
        assert_eq!(parse_colour("#12"), None);
        assert_eq!(parse_colour("sunset"), None);
        // The table is sorted and holds no repeats.
        assert!(CSS_COLOURS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn points_of_interest_and_land_use_type_their_buildings() {
        assert_eq!(
            PoiKind::of(
                "place_of_worship",
                Some("muslim"),
                Some("Süleymaniye Camii")
            ),
            Some(PoiKind::Worship(Religion::Muslim))
        );
        assert_eq!(
            PoiKind::of("place_of_worship", None, None),
            Some(PoiKind::Worship(Religion::Christian))
        );
        assert_eq!(
            PoiKind::of("attraction", Some("attraction"), Some("Gróttuviti")),
            Some(PoiKind::Lighthouse)
        );
        assert_eq!(
            PoiKind::of("museum", Some("museum"), Some("Point Pinos Lighthouse")),
            Some(PoiKind::Lighthouse)
        );
        // A bank, a park or a hotel named for a lighthouse is not one.
        assert_eq!(
            PoiKind::of("bank", Some("bank"), Some("Lighthouse Bank")),
            None
        );
        assert_eq!(
            PoiKind::of("park", Some("park"), Some("Lighthouse Field State Beach")),
            None
        );
        assert_eq!(
            PoiKind::of("lodging", Some("hotel"), Some("Lighthouse Lodge")),
            None
        );
        assert_eq!(
            PoiKind::of("harbor", Some("marina"), None),
            Some(PoiKind::Harbour)
        );
        assert_eq!(PoiKind::of("fuel", Some("charging_station"), None), None);
        assert!(is_lighthouse_name("Phare de Cordouan") && is_lighthouse_name("Lindesnes fyr"));
        assert!(!is_lighthouse_name("Lighthouse Avenue"));

        assert_eq!(Usage::of_landuse("industrial"), Some(Usage::Industrial));
        assert_eq!(Usage::of_landuse("retail"), Some(Usage::Commercial));
        assert_eq!(Usage::of_landuse("neighbourhood"), None);
        // A school within a residential district is the school; a hole in a zone is not the zone.
        let tags = MapTags {
            zones: vec![
                Zone::new(
                    Usage::Residential,
                    square(0.0, 0.0, 500.0),
                    vec![square(400.0, 400.0, 50.0)],
                ),
                Zone::new(Usage::Civic, square(100.0, 100.0, 100.0), vec![]),
            ],
            pois: vec![Poi {
                at: [255.0, 255.0],
                kind: PoiKind::Worship(Religion::Christian),
            }],
            ..MapTags::default()
        };
        assert_eq!(tags.usage_at([50.0, 50.0]), Usage::Residential);
        assert_eq!(tags.usage_at([150.0, 150.0]), Usage::Civic);
        assert_eq!(tags.usage_at([420.0, 420.0]), Usage::Unknown);
        let flat = |_: [f32; 2]| 5.0f32;
        let church = tags.setting(&square(240.0, 240.0, 30.0), &flat);
        assert_eq!(church.poi, Some(PoiKind::Worship(Religion::Christian)));
        assert_eq!(church.usage, Usage::Residential);
        assert!(!church.near_sea && church.pier.is_none());
        let house = tags.setting(&square(300.0, 300.0, 10.0), &flat);
        assert_eq!(house.poi, None);
        // The sea 100 m west of a round footprint (only a round one asks).
        let coast = |p: [f32; 2]| if p[0] < 200.0 { -5.0 } else { 5.0 };
        let tank = geometry::regular([265.0, 305.0], 6.0, 12, 0.0);
        assert!(tags.setting(&tank, &coast).near_sea);
        assert!(!tags.setting(&square(260.0, 300.0, 10.0), &coast).near_sea);
    }

    #[test]
    fn outlines_lend_their_heights_and_add_what_the_footprints_miss() {
        let fp = |ring: Vec<[f32; 2]>, height: Option<f32>| Footprint {
            ring,
            height,
            kind: None,
            colour: None,
            part: None,
        };
        let mut footprints = vec![
            // No height: OSM's 30 m taken. Capped at 35 m: OSM's 80 m taken. A measured 12 m kept over OSM's 9.
            fp(square(100.0, 100.0, 20.0), None),
            fp(square(200.0, 100.0, 20.0), Some(34.8)),
            fp(square(300.0, 100.0, 20.0), Some(12.0)),
        ];
        let outline = |ring: Vec<[f32; 2]>, height: Option<f32>| Outline {
            ring,
            height,
            min_height: 0.0,
            colour: Some([200, 0, 0]),
            hide_3d: false,
        };
        let outlines = vec![
            outline(square(99.0, 99.0, 22.0), Some(30.0)),
            outline(square(199.0, 99.0, 22.0), Some(80.0)),
            outline(square(301.0, 101.0, 18.0), Some(9.0)),
            // A church Microsoft missed, and a part floating above the ground, and an outline drawn by its parts.
            outline(square(500.0, 500.0, 30.0), None),
            Outline {
                min_height: 20.0,
                ..outline(square(600.0, 600.0, 10.0), Some(40.0))
            },
            Outline {
                hide_3d: true,
                ..outline(square(700.0, 700.0, 10.0), Some(10.0))
            },
        ];
        merge_outlines(&mut footprints, &outlines, &[]);
        assert_eq!(footprints.len(), 5);
        assert_eq!(footprints[0].height, Some(30.0));
        assert_eq!(footprints[1].height, Some(80.0));
        assert_eq!(footprints[2].height, Some(12.0));
        assert_eq!(footprints[0].colour, Some([200, 0, 0]));
        assert_eq!(footprints[3].ring, square(500.0, 500.0, 30.0));
        assert_eq!(footprints[3].height, None);
        assert_eq!(footprints[3].part, None);
        // The part floating over open ground stands as mapped, from its base.
        assert_eq!(footprints[4].ring, square(600.0, 600.0, 10.0));
        assert_eq!(
            footprints[4].part,
            Some(Part {
                base: 20.0,
                middle: [605.0, 605.0]
            })
        );
    }

    /// Taipei 101 as OpenStreetMap draws it: the complex's outline drawn by its parts, the shaft on the ground to 386 m,
    /// a slab, the crown, the top and the spire stacked on it, a podium on the ground beside it; and Microsoft's
    /// footprint of the tower, capped at 34 m.
    fn taipei_101() -> (Vec<Footprint>, Vec<Outline>) {
        let part = |ring: Vec<[f32; 2]>, base: f32, top: f32| Outline {
            ring,
            height: Some(top),
            min_height: base,
            colour: None,
            hide_3d: false,
        };
        let outlines = vec![
            Outline {
                hide_3d: true,
                ..part(square(400.0, 400.0, 125.0), 0.0, 508.0)
            },
            part(square(430.0, 430.0, 53.0), 0.0, 386.0),
            part(square(434.0, 434.0, 45.0), 386.0, 391.0),
            part(square(445.0, 445.0, 23.0), 390.0, 439.0),
            part(square(450.0, 450.0, 13.0), 439.0, 448.0),
            part(square(455.8, 455.8, 1.4), 448.0, 508.0),
            part(square(490.0, 405.0, 30.0), 0.0, 25.0),
        ];
        let footprints = vec![Footprint {
            ring: square(431.0, 431.0, 51.0),
            height: Some(34.5),
            ..Footprint::default()
        }];
        (footprints, outlines)
    }

    #[test]
    fn a_tower_mapped_as_parts_stands_as_its_parts() {
        let (mut footprints, outlines) = taipei_101();
        merge_outlines(&mut footprints, &outlines, &[]);
        // Microsoft's footprint left out, the outline drawn by its parts, every part standing.
        assert_eq!(footprints.len(), 6, "{footprints:?}");
        let middle = [462.5, 462.5];
        let parts: Vec<(f32, Option<f32>)> = footprints
            .iter()
            .map(|f| {
                let p = f.part.expect("a part");
                assert_eq!(p.middle, middle, "each read at the complex's middle");
                (p.base, f.height)
            })
            .collect();
        assert_eq!(
            parts,
            vec![
                (0.0, Some(386.0)),
                (386.0, Some(391.0)),
                (390.0, Some(439.0)),
                (439.0, Some(448.0)),
                (448.0, Some(508.0)),
                (0.0, Some(25.0)),
            ]
        );
        assert!(footprints.iter().filter(|f| f.raised()).count() == 4);
    }

    #[test]
    fn a_footprint_under_a_raised_part_gives_way_to_the_parts() {
        // No outline drawn by parts: a building on the ground with no height and a part over it from 40 to 90 m, over
        // Microsoft's footprint (24 m). The footprint is left out; the building takes its height, the part stands on it.
        let part = |ring: Vec<[f32; 2]>, base: f32, top: Option<f32>| Outline {
            ring,
            height: top,
            min_height: base,
            colour: None,
            hide_3d: false,
        };
        let mut footprints = vec![
            Footprint {
                ring: square(100.0, 100.0, 40.0),
                height: Some(24.0),
                ..Footprint::default()
            },
            // Beside them a footprint under a raised part with nothing on the ground: it stands, at the part's top.
            Footprint {
                ring: square(300.0, 300.0, 20.0),
                ..Footprint::default()
            },
        ];
        let outlines = vec![
            part(square(99.0, 99.0, 42.0), 0.0, None),
            part(square(110.0, 110.0, 20.0), 40.0, Some(90.0)),
            part(square(301.0, 301.0, 18.0), 12.0, Some(30.0)),
        ];
        merge_outlines(&mut footprints, &outlines, &[]);
        assert_eq!(footprints.len(), 3, "{footprints:?}");
        assert_eq!(footprints[0].ring, square(300.0, 300.0, 20.0));
        assert_eq!(footprints[0].height, Some(30.0));
        assert_eq!(footprints[0].part, None);
        let [_, under, raised] = &footprints[..] else {
            unreachable!()
        };
        assert_eq!(under.ring, square(99.0, 99.0, 42.0));
        assert_eq!(under.height, Some(24.0));
        assert_eq!(under.part.map(|p| p.base), Some(0.0));
        assert_eq!(raised.ring, square(110.0, 110.0, 20.0));
        assert_eq!(
            raised.part.map(|p| (p.base, p.middle)),
            Some((40.0, [120.0, 120.0]))
        );
        assert_eq!(under.part.map(|p| p.middle), Some([120.0, 120.0]));
    }

    #[test]
    fn a_part_that_cannot_stand_leaves_microsofts_footprint_standing() {
        // A building drawn by its parts holding Microsoft's footprint (18 m) and one part from 12 m with no height: the
        // part stands nowhere, so the footprint stands in its place.
        let mut footprints = vec![Footprint {
            ring: square(110.0, 110.0, 30.0),
            height: Some(18.0),
            ..Footprint::default()
        }];
        let outlines = vec![
            Outline {
                ring: square(100.0, 100.0, 50.0),
                height: None,
                min_height: 0.0,
                colour: None,
                hide_3d: true,
            },
            Outline {
                ring: square(112.0, 112.0, 26.0),
                height: None,
                min_height: 12.0,
                colour: None,
                hide_3d: false,
            },
        ];
        merge_outlines(&mut footprints, &outlines, &[]);
        assert_eq!(footprints.len(), 1, "{footprints:?}");
        assert_eq!(footprints[0].ring, square(110.0, 110.0, 30.0));
        assert_eq!(
            (footprints[0].height, footprints[0].part),
            (Some(18.0), None)
        );
    }

    #[test]
    fn a_towers_parts_across_a_chunks_edge_keep_one_middle() {
        // A building drawn by its parts about theatre (995, 430): its outline and shaft (on the ground to 120 m) in
        // chunk (0, 0), its crown (from 120 m) about (1005, 430) in chunk (1, 0); the ground rising 0.1 m a metre east.
        let outline = |x: f32, side: f32, base: f32, top: f32, hide_3d: bool| Outline {
            ring: square(x, 400.0 + (60.0 - side) * 0.5, side),
            height: Some(top),
            min_height: base,
            colour: None,
            hide_3d,
        };
        // Each chunk's own outlines and those about it, in its own frame (x from its corner).
        let shift = |o: &Outline, dx: f32| Outline {
            ring: o.ring.iter().map(|p| [p[0] + dx, p[1]]).collect(),
            ..o.clone()
        };
        let building = outline(965.0, 60.0, 0.0, 160.0, true);
        let shaft = outline(975.0, 40.0, 0.0, 120.0, false);
        let crown = outline(995.0, 20.0, 120.0, 160.0, false);
        let mut west = Vec::new();
        merge_outlines(
            &mut west,
            &[building.clone(), shaft.clone()],
            std::slice::from_ref(&crown),
        );
        let mut east = Vec::new();
        merge_outlines(
            &mut east,
            &[shift(&crown, -1000.0)],
            &[shift(&building, -1000.0), shift(&shaft, -1000.0)],
        );
        // Each part stands once, in its own chunk, at the building's middle.
        assert_eq!((west.len(), east.len()), (1, 1), "{west:?} {east:?}");
        let (shaft, crown) = (&west[0], &east[0]);
        let middle = |f: &Footprint| f.part.expect("a part").middle;
        assert!((middle(shaft)[0] - 995.0).abs() < 1e-3, "{shaft:?}");
        assert!((middle(crown)[0] + 5.0).abs() < 1e-3, "{crown:?}");
        // Planned in their chunks: one colour and hash, and the crown's walls start at the shaft's top.
        let plan = |f: &Footprint, chunk: i32| {
            let ground = move |p: [f32; 2]| 5.0 + 0.1 * (p[0] + 1000.0 * chunk as f32);
            crate::buildings::plan_building(
                f,
                [chunk, 0],
                0.6,
                crate::buildings::Region::Temperate,
                crate::buildings::Period::Today,
                &Setting::default(),
                &ground,
            )
            .expect("it stands")
        };
        let (shaft, crown) = (plan(shaft, 0), plan(crown, 1));
        assert_eq!((shaft.seed, shaft.wall), (crown.seed, crown.wall));
        let top = shaft.ground[1] + shaft.height;
        assert!(
            (crate::buildings::mesh::foot_of(&crown) - top).abs() < 1e-3,
            "{} {top}",
            crate::buildings::mesh::foot_of(&crown)
        );
    }

    #[test]
    fn quays_run_along_industrial_land_at_the_water() {
        // Industrial land from x = 100 to 400, the sea west of x = 100 (by the ground, no water mapped).
        let tags = MapTags {
            zones: vec![Zone::new(
                Usage::Industrial,
                vec![
                    [100.0, 100.0],
                    [400.0, 100.0],
                    [400.0, 500.0],
                    [100.0, 500.0],
                ],
                vec![],
            )],
            ..MapTags::default()
        };
        let ground = |p: [f32; 2]| if p[0] < 100.0 { -6.0 } else { 3.0 };
        let quays = tags.quays(&ground, 60.0);
        assert_eq!(quays.len(), 1, "{quays:?}");
        let q = quays[0];
        assert!(q.length() > 360.0, "{}", q.length());
        assert!((q.out[0] + 1.0).abs() < 1e-3 && q.out[1].abs() < 1e-3);
        assert!((q.a[0] - 100.0).abs() < 1e-3);
        // Water mapped: the sea is the map's, not the ground's.
        let mapped = MapTags {
            water: vec![WaterArea::new(square(-500.0, -500.0, 550.0), vec![])],
            ..tags.clone()
        };
        assert!(mapped.quays(&ground, 60.0).is_empty());
        assert!(mapped.is_water([0.0, 0.0], &ground) && !mapped.is_water([0.0, 300.0], &ground));
    }

    #[test]
    fn a_street_through_a_lot_is_found() {
        let tags = MapTags {
            roads: vec![Road {
                a: [0.0, 50.0],
                b: [200.0, 50.0],
                half: 3.0,
            }],
            ..MapTags::default()
        };
        // A 12 m house across the street, one beside it, one a lot back.
        assert!(tags.on_road(&square(40.0, 44.0, 12.0)));
        assert!(tags.on_road(&square(40.0, 50.0, 12.0)));
        assert!(!tags.on_road(&square(40.0, 56.0, 12.0)));
        assert!(!tags.on_road(&square(40.0, 70.0, 12.0)));
    }
}
