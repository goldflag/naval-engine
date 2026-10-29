//! The OpenMapTiles schema's layers the game reads (https://openmaptiles.org/schema/, version 3.x as OpenFreeMap serves
//! it), typed: `transportation`, `waterway`, `water`, `landuse`, `landcover`, `building`, `poi` and `aeroway`. Classes stay strings as the
//! schema names them (a newer schema's new class reaches the caller rather than being dropped); the tags every layer
//! uses for drawing are fields.
//!
//! Geometry is generic over its point: [`Features::from_tile`] gives each point as fractions of the tile (0 at its
//! north-west corner, 1 at the next tile's; a buffer's points lie a little past either end, about 1.6% at OpenFreeMap's
//! 64 units of 4096), and [`Features::map`] carries the same features to any other frame (the client's theatre metres).

use crate::mvt::{Geometry, Layer, Tile};

/// A road or rail way's crossing of something else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Brunnel {
    #[default]
    None,
    Bridge,
    Tunnel,
    Ford,
}

impl Brunnel {
    fn of(layer: &Layer, f: &crate::mvt::Feature) -> Self {
        match layer.str(f, "brunnel") {
            Some("bridge") => Self::Bridge,
            Some("tunnel") => Self::Tunnel,
            Some("ford") => Self::Ford,
            _ => Self::None,
        }
    }
}

/// `transportation`'s `surface`: OpenMapTiles folds OSM's surfaces into two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadSurface {
    Paved,
    Unpaved,
}

/// A way of `transportation`: roads (`motorway`, `trunk`, `primary`, `secondary`, `tertiary`, `minor`, `service`,
/// `track`, `path`, `raceway`, `busway`, each with `_construction` while being built), railways (`rail`, `transit`,
/// with `subclass` `rail`, `narrow_gauge`, `preserved`, `funicular`, `light_rail`, `subway`, `tram`, `monorail`),
/// `ferry`, `cable_car`, `bridge` and `pier` areas.
#[derive(Clone, Debug, PartialEq)]
pub struct Transportation<P> {
    pub class: String,
    pub subclass: Option<String>,
    pub brunnel: Brunnel,
    pub surface: Option<RoadSurface>,
    /// OSM's `layer` (bridges above, tunnels below), 0 when absent.
    pub layer: i32,
    pub ramp: bool,
    /// 1 one way along the line, -1 against it, 0 both.
    pub oneway: i8,
    pub expressway: bool,
    /// `service`'s kind (`driveway`, `parking_aisle`, `alley`, `siding`, `spur`, `yard`, ...).
    pub service: Option<String>,
    /// The road network (`us-interstate`, `gb-motorway`, ...).
    pub network: Option<String>,
    pub geometry: Geometry<P>,
}

/// A waterway line: `river`, `stream`, `canal`, `drain`, `ditch`. Rivers wide enough are mapped as `water` areas too
/// (class `river`), which this line runs down the middle of.
#[derive(Clone, Debug, PartialEq)]
pub struct Waterway<P> {
    pub class: String,
    pub brunnel: Brunnel,
    pub intermittent: bool,
    pub geometry: Geometry<P>,
}

/// A water area: `ocean`, `lake`, `river` (a riverbank), `pond`, `dock`, `swimming_pool`.
#[derive(Clone, Debug, PartialEq)]
pub struct Water<P> {
    pub class: String,
    pub brunnel: Brunnel,
    pub intermittent: bool,
    /// The OSM id the schema carries for lakes, when it does.
    pub id: Option<i64>,
    pub geometry: Geometry<P>,
}

/// Land use: `residential`, `commercial`, `industrial`, `retail`, `railway`, `cemetery`, `military`, `quarry`,
/// `farmland` (at low zooms), `stadium`, `school`, `university`, `hospital`, `garages`, `dam`, ...
#[derive(Clone, Debug, PartialEq)]
pub struct Landuse<P> {
    pub class: String,
    pub geometry: Geometry<P>,
}

/// Land cover: `class` `farmland`, `ice`, `wood`, `rock`, `grass`, `wetland`, `sand`, with OSM's own value in
/// `subclass` (`forest`, `meadow`, `scrub`, `beach`, `vineyard`, `orchard`, `glacier`, `bare_rock`, ...).
#[derive(Clone, Debug, PartialEq)]
pub struct Landcover<P> {
    pub class: String,
    pub subclass: Option<String>,
    pub geometry: Geometry<P>,
}

/// A building's footprint and heights: `render_height` (metres to its top, from OSM's `height` or `building:levels`,
/// else the schema's default) and `render_min_height` (where a part starts above the ground), `colour` (a CSS colour
/// from OSM's `building:colour` or the schema's default by its material) and `hide_3d` (a part drawn by another).
#[derive(Clone, Debug, PartialEq)]
pub struct Building<P> {
    pub id: Option<u64>,
    pub render_height: Option<f64>,
    pub render_min_height: Option<f64>,
    pub colour: Option<String>,
    pub hide_3d: bool,
    pub geometry: Geometry<P>,
}

/// A point of interest: `class` the schema's grouping (`place_of_worship`, `harbor`, `ferry_terminal`, `fuel`,
/// `lodging`, `museum`, `attraction`, `shop`, ...) and `subclass` OSM's own value (for `place_of_worship` the religion:
/// `christian`, `muslim`, `jewish`, `buddhist`, `hindu`, ...; for `harbor` `marina` or `dock`). The schema carries no
/// `man_made` (no lighthouses, chimneys or cranes): a lighthouse is known here by its name alone. A point of a building
/// mapped as an area stands on the building (the schema takes a point on its surface).
#[derive(Clone, Debug, PartialEq)]
pub struct Poi<P> {
    pub class: String,
    pub subclass: Option<String>,
    /// The local name (`name`).
    pub name: Option<String>,
    /// The schema's importance within the tile, 1 the most.
    pub rank: Option<i32>,
    /// OSM's `layer` and `level` (a shop on a building's upper floor), 0 when absent.
    pub layer: i32,
    pub level: i32,
    pub geometry: Geometry<P>,
}

/// An aerodrome's part: `class` `runway` and `taxiway` (centre lines, or areas where mapped so), `apron`, `helipad`,
/// `aerodrome` (the field's outline), `hangar`, `gate` (points), `heliport`; `reference` OSM's `ref`: a runway's
/// designators (`07L/25R`, `06 / 24`), a taxiway's letter, a gate's number. OSM's widths and surfaces are not in the
/// schema.
#[derive(Clone, Debug, PartialEq)]
pub struct Aeroway<P> {
    pub class: String,
    pub reference: Option<String>,
    pub geometry: Geometry<P>,
}

/// A tile's features in the layers above.
#[derive(Clone, Debug, PartialEq)]
pub struct Features<P> {
    pub transportation: Vec<Transportation<P>>,
    pub waterway: Vec<Waterway<P>>,
    pub water: Vec<Water<P>>,
    pub landuse: Vec<Landuse<P>>,
    pub landcover: Vec<Landcover<P>>,
    pub building: Vec<Building<P>>,
    pub poi: Vec<Poi<P>>,
    pub aeroway: Vec<Aeroway<P>>,
}

impl<P> Default for Features<P> {
    fn default() -> Self {
        Self {
            transportation: Vec::new(),
            waterway: Vec::new(),
            water: Vec::new(),
            landuse: Vec::new(),
            landcover: Vec::new(),
            building: Vec::new(),
            poi: Vec::new(),
            aeroway: Vec::new(),
        }
    }
}

/// Fractions of the tile.
pub type TilePoint = [f64; 2];

fn class(layer: &Layer, f: &crate::mvt::Feature) -> String {
    layer.str(f, "class").unwrap_or_default().to_owned()
}

fn owned(layer: &Layer, f: &crate::mvt::Feature, key: &str) -> Option<String> {
    layer.str(f, key).map(str::to_owned)
}

fn flag(layer: &Layer, f: &crate::mvt::Feature, key: &str) -> bool {
    layer.number(f, key).is_some_and(|v| v != 0.0)
}

fn shape(layer: &Layer, f: &crate::mvt::Feature) -> Geometry<TilePoint> {
    let extent = f64::from(layer.extent.max(1));
    f.geometry
        .map(&mut |p| [f64::from(p[0]) / extent, f64::from(p[1]) / extent])
}

impl Features<TilePoint> {
    /// The typed features of a decoded tile, points as fractions of the tile.
    pub fn from_tile(tile: &Tile) -> Self {
        let mut out = Self::default();
        for layer in &tile.layers {
            let l = layer;
            match layer.name.as_str() {
                "transportation" => {
                    out.transportation
                        .extend(l.features.iter().map(|f| Transportation {
                            class: class(l, f),
                            subclass: owned(l, f, "subclass"),
                            brunnel: Brunnel::of(l, f),
                            surface: match l.str(f, "surface") {
                                Some("paved") => Some(RoadSurface::Paved),
                                Some("unpaved") => Some(RoadSurface::Unpaved),
                                _ => None,
                            },
                            layer: l.number(f, "layer").unwrap_or(0.0) as i32,
                            ramp: flag(l, f, "ramp"),
                            oneway: l.number(f, "oneway").unwrap_or(0.0).clamp(-1.0, 1.0) as i8,
                            expressway: flag(l, f, "expressway"),
                            service: owned(l, f, "service"),
                            network: owned(l, f, "network"),
                            geometry: shape(l, f),
                        }))
                }
                "waterway" => out.waterway.extend(l.features.iter().map(|f| Waterway {
                    class: class(l, f),
                    brunnel: Brunnel::of(l, f),
                    intermittent: flag(l, f, "intermittent"),
                    geometry: shape(l, f),
                })),
                "water" => out.water.extend(l.features.iter().map(|f| Water {
                    class: class(l, f),
                    brunnel: Brunnel::of(l, f),
                    intermittent: flag(l, f, "intermittent"),
                    id: l.number(f, "id").map(|v| v as i64),
                    geometry: shape(l, f),
                })),
                "landuse" => out.landuse.extend(l.features.iter().map(|f| Landuse {
                    class: class(l, f),
                    geometry: shape(l, f),
                })),
                "landcover" => out.landcover.extend(l.features.iter().map(|f| Landcover {
                    class: class(l, f),
                    subclass: owned(l, f, "subclass"),
                    geometry: shape(l, f),
                })),
                "building" => out.building.extend(l.features.iter().map(|f| Building {
                    id: f.id,
                    render_height: l.number(f, "render_height"),
                    render_min_height: l.number(f, "render_min_height"),
                    colour: owned(l, f, "colour"),
                    hide_3d: flag(l, f, "hide_3d"),
                    geometry: shape(l, f),
                })),
                "poi" => out.poi.extend(l.features.iter().map(|f| Poi {
                    class: class(l, f),
                    subclass: owned(l, f, "subclass"),
                    name: owned(l, f, "name"),
                    rank: l.number(f, "rank").map(|v| v as i32),
                    layer: l.number(f, "layer").unwrap_or(0.0) as i32,
                    level: l.number(f, "level").unwrap_or(0.0) as i32,
                    geometry: shape(l, f),
                })),
                "aeroway" => out.aeroway.extend(l.features.iter().map(|f| Aeroway {
                    class: class(l, f),
                    reference: owned(l, f, "ref"),
                    geometry: shape(l, f),
                })),
                _ => {}
            }
        }
        out
    }
}

impl<P> Features<P> {
    /// The same features with every point mapped by `f` (into the theatre's metres, say).
    pub fn map<Q>(&self, f: &mut impl FnMut(&P) -> Q) -> Features<Q> {
        Features {
            transportation: self
                .transportation
                .iter()
                .map(|t| Transportation {
                    class: t.class.clone(),
                    subclass: t.subclass.clone(),
                    brunnel: t.brunnel,
                    surface: t.surface,
                    layer: t.layer,
                    ramp: t.ramp,
                    oneway: t.oneway,
                    expressway: t.expressway,
                    service: t.service.clone(),
                    network: t.network.clone(),
                    geometry: t.geometry.map(f),
                })
                .collect(),
            waterway: self
                .waterway
                .iter()
                .map(|w| Waterway {
                    class: w.class.clone(),
                    brunnel: w.brunnel,
                    intermittent: w.intermittent,
                    geometry: w.geometry.map(f),
                })
                .collect(),
            water: self
                .water
                .iter()
                .map(|w| Water {
                    class: w.class.clone(),
                    brunnel: w.brunnel,
                    intermittent: w.intermittent,
                    id: w.id,
                    geometry: w.geometry.map(f),
                })
                .collect(),
            landuse: self
                .landuse
                .iter()
                .map(|l| Landuse {
                    class: l.class.clone(),
                    geometry: l.geometry.map(f),
                })
                .collect(),
            landcover: self
                .landcover
                .iter()
                .map(|l| Landcover {
                    class: l.class.clone(),
                    subclass: l.subclass.clone(),
                    geometry: l.geometry.map(f),
                })
                .collect(),
            building: self
                .building
                .iter()
                .map(|b| Building {
                    id: b.id,
                    render_height: b.render_height,
                    render_min_height: b.render_min_height,
                    colour: b.colour.clone(),
                    hide_3d: b.hide_3d,
                    geometry: b.geometry.map(f),
                })
                .collect(),
            poi: self
                .poi
                .iter()
                .map(|p| Poi {
                    class: p.class.clone(),
                    subclass: p.subclass.clone(),
                    name: p.name.clone(),
                    rank: p.rank,
                    layer: p.layer,
                    level: p.level,
                    geometry: p.geometry.map(f),
                })
                .collect(),
            aeroway: self
                .aeroway
                .iter()
                .map(|a| Aeroway {
                    class: a.class.clone(),
                    reference: a.reference.clone(),
                    geometry: a.geometry.map(f),
                })
                .collect(),
        }
    }

    /// Points in every layer (a measure of the tile's size).
    pub fn point_count(&self) -> usize {
        self.transportation
            .iter()
            .map(|t| t.geometry.points().count())
            .chain(self.waterway.iter().map(|w| w.geometry.points().count()))
            .chain(self.water.iter().map(|w| w.geometry.points().count()))
            .chain(self.landuse.iter().map(|l| l.geometry.points().count()))
            .chain(self.landcover.iter().map(|l| l.geometry.points().count()))
            .chain(self.building.iter().map(|b| b.geometry.points().count()))
            .chain(self.poi.iter().map(|p| p.geometry.points().count()))
            .chain(self.aeroway.iter().map(|a| a.geometry.points().count()))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvt::encode::*;
    use crate::mvt::{Polygon, decode};

    #[test]
    fn the_schemas_layers_come_out_typed() {
        let road = feature(
            1,
            &[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)],
            2,
            &commands(&[(&[[0, 0], [2048, 4096]], false)]),
        );
        let rail = feature(
            2,
            &[(0, 5), (5, 6)],
            2,
            &commands(&[(&[[10, 10], [20, 20]], false)]),
        );
        let transportation = layer(
            "transportation",
            &["class", "brunnel", "surface", "layer", "oneway", "subclass"],
            &[
                string_value("primary"),
                string_value("bridge"),
                string_value("unpaved"),
                sint_value(1),
                sint_value(-1),
                string_value("rail"),
                string_value("narrow_gauge"),
            ],
            &[road, rail],
        );
        let square: &[[i32; 2]] = &[[0, 0], [4096, 0], [4096, 4096], [0, 4096]];
        let lake = feature(3, &[(0, 0), (1, 1)], 3, &commands(&[(square, true)]));
        let water = layer(
            "water",
            &["class", "id"],
            &[string_value("lake"), double_value(42.0)],
            &[lake],
        );
        let house = feature(4, &[(0, 0), (1, 1)], 3, &commands(&[(square, true)]));
        let building = layer(
            "building",
            &["render_height", "render_min_height"],
            &[double_value(12.0), double_value(3.0)],
            &[house],
        );
        let river = feature(5, &[(0, 0)], 2, &commands(&[(&[[0, 0], [1, 1]], false)]));
        let waterway = layer("waterway", &["class"], &[string_value("river")], &[river]);
        // Points of interest: a mosque, a marina on the first floor and a lighthouse known by its name alone.
        let mosque = feature(
            6,
            &[(0, 0), (1, 1), (2, 2), (3, 3)],
            1,
            &commands(&[(&[[1024, 3072]], false)]),
        );
        let marina = feature(
            7,
            &[(0, 4), (1, 5), (4, 6)],
            1,
            &commands(&[(&[[4096, 0]], false)]),
        );
        let light = feature(
            8,
            &[(0, 7), (1, 7), (2, 8)],
            1,
            &commands(&[(&[[10, 20]], false)]),
        );
        let poi = layer(
            "poi",
            &["class", "subclass", "name", "rank", "level"],
            &[
                string_value("place_of_worship"),
                string_value("muslim"),
                string_value("Süleymaniye Camii"),
                sint_value(2),
                string_value("harbor"),
                string_value("marina"),
                sint_value(1),
                string_value("attraction"),
                string_value("Gróttuviti"),
            ],
            &[mosque, marina, light],
        );
        // An aerodrome's runway (a centre line) and apron.
        let runway = feature(
            9,
            &[(0, 0), (1, 1)],
            2,
            &commands(&[(&[[1024, 2048], [3072, 1024]], false)]),
        );
        let apron = feature(10, &[(0, 2)], 3, &commands(&[(square, true)]));
        let aeroway = layer(
            "aeroway",
            &["class", "ref"],
            &[
                string_value("runway"),
                string_value("07L/25R"),
                string_value("apron"),
            ],
            &[runway, apron],
        );
        let t = decode(&tile(&[
            transportation,
            water,
            building,
            waterway,
            poi,
            aeroway,
        ]))
        .unwrap();
        let f = Features::from_tile(&t);

        let r = &f.transportation[0];
        assert_eq!(r.class, "primary");
        assert_eq!(r.brunnel, Brunnel::Bridge);
        assert_eq!(r.surface, Some(RoadSurface::Unpaved));
        assert_eq!((r.layer, r.oneway), (1, -1));
        assert_eq!(
            r.geometry,
            Geometry::Lines(vec![vec![[0.0, 0.0], [0.5, 1.0]]])
        );
        assert_eq!(f.transportation[1].class, "rail");
        assert_eq!(
            f.transportation[1].subclass.as_deref(),
            Some("narrow_gauge")
        );
        assert_eq!(f.water[0].class, "lake");
        assert_eq!(f.water[0].id, Some(42));
        let b = &f.building[0];
        assert_eq!(
            (b.render_height, b.render_min_height),
            (Some(12.0), Some(3.0))
        );
        assert_eq!(b.id, Some(4));
        assert_eq!(f.waterway[0].class, "river");
        let [mosque, marina, light] = [&f.poi[0], &f.poi[1], &f.poi[2]];
        assert_eq!(
            (mosque.class.as_str(), mosque.subclass.as_deref()),
            ("place_of_worship", Some("muslim"))
        );
        assert_eq!(mosque.name.as_deref(), Some("Süleymaniye Camii"));
        assert_eq!((mosque.rank, mosque.level, mosque.layer), (Some(2), 0, 0));
        assert_eq!(mosque.geometry, Geometry::Points(vec![[0.25, 0.75]]));
        assert_eq!(
            (
                marina.class.as_str(),
                marina.subclass.as_deref(),
                marina.level
            ),
            ("harbor", Some("marina"), 1)
        );
        assert_eq!(marina.name, None);
        assert_eq!(marina.geometry, Geometry::Points(vec![[1.0, 0.0]]));
        assert_eq!(
            (
                light.class.as_str(),
                light.subclass.as_deref(),
                light.name.as_deref()
            ),
            ("attraction", Some("attraction"), Some("Gróttuviti"))
        );
        let [runway, apron] = [&f.aeroway[0], &f.aeroway[1]];
        assert_eq!(
            (runway.class.as_str(), runway.reference.as_deref()),
            ("runway", Some("07L/25R"))
        );
        assert_eq!(
            runway.geometry,
            Geometry::Lines(vec![vec![[0.25, 0.5], [0.75, 0.25]]])
        );
        assert_eq!(
            (apron.class.as_str(), apron.reference.as_deref()),
            ("apron", None)
        );
        assert_eq!(f.point_count(), 2 + 2 + 4 + 4 + 2 + 3 + 2 + 4);

        // Mapped into another frame, point by point.
        let m = f.map(&mut |p| [p[0] * 1000.0, -p[1] * 1000.0]);
        assert_eq!(m.poi[0].geometry, Geometry::Points(vec![[250.0, -750.0]]));
        assert_eq!(m.poi[2].name.as_deref(), Some("Gróttuviti"));
        assert_eq!(
            m.water[0].geometry,
            Geometry::Polygons(vec![Polygon {
                exterior: vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, -1000.0], [0.0, -1000.0]],
                holes: vec![],
            }])
        );
    }
}
