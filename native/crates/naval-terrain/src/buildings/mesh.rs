//! A chunk's buildings as one indexed mesh: each planned building extruded from just below the lowest ground under it
//! to its eaves, under a flat, gabled, hipped or sawtooth roof (a tank under its cone), with its extras (a church's
//! tower, a dome, minarets, chimneys, a lighthouse, a quay's cranes; [`super::parts`]) (`Lod::Full`), or as a box under
//! a flat top with its tall extras simplified (`Lod::Box`); a pier's deck over the water.
//!
//! Every face is flat-shaded (its own vertices), round parts smooth. A wall's facade coordinates are window bays along
//! it (whole bays from corner to corner, about the building's bay wide) and storeys up from the mean ground, so the
//! shader lays its windows out per wall without knowing the wall; a roof's are metres down its slope from the ridge and
//! along it, for the tiles.

use half::f16;

use super::clutter::{Clutter, SHAPES, Shape, roof_clutter, sort_by_shape};
use super::eaves::{Eaves, dormers_of, eaves_of, parapet_of, place_hash};
use super::far::Mass;
use super::geometry::{self, signed_area2};
use super::grounds::{self, Footings, Roads};
use super::parcels;
use super::tags::{Landmark, LandmarkKind, MapTags, Pier, PoiKind, Quay, Usage};
use super::thinning::{self, Thinning};
use super::{
    Building, ChunkKey, DensityGrid, Extra, Footprint, Kind, Lod, Period, Region, Roof, hash01,
    plan_building,
};

/// One vertex, 24 bytes (`buildings.wgsl`'s `VertexIn`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BuildingVertex {
    /// Chunk-local x, z (m from the chunk's corner) and height above the sea (m).
    pub position: [f32; 3],
    /// The face's normal (snorm8 x, y, z); w: the building's window rows.
    pub normal: [i8; 4],
    /// Facade coordinates as f16: a wall's bay and storey, a roof's slope and run (m), a banded part's band.
    pub facade: [u16; 2],
    /// The walls' palette entry, the roof's, the building's hash, and its kind (bits 0-3), the face's part (bits 4-6:
    /// [`PART_WALL`] ... [`PART_DOME`]) and its detail (bit 7: shutters, balconies, a curtain wall, a lit lantern).
    pub style: [u8; 4],
}

/// A wall with windows laid out by its kind.
pub const PART_WALL: u8 = 0;
/// A roof in the roof's colour: tile courses down a slope, patches on the flat.
pub const PART_ROOF: u8 = 1;
/// A plain wall in the walls' colour: a gable, a tank's side, a drum, a pier's edge.
pub const PART_GABLE: u8 = 2;
/// A tower painted in bands: the walls' colour below the bands, then the roof's colour and white by turns (a
/// lighthouse, a stack's aviation marks).
pub const PART_BANDED: u8 = 3;
/// Glass: a lantern (lit at night when its detail bit is set), a sawtooth's north lights.
pub const PART_GLASS: u8 = 4;
/// Painted steel in the walls' entry: cranes, a gallery's rail.
pub const PART_STEEL: u8 = 5;
/// A smooth roof in the roof's colour, no courses: a spire, a dome, a cap.
pub const PART_DOME: u8 = 6;
/// Leaves in the walls' entry (the colour cube's greens): a clipped hedge ([`super::grounds`], clutter only).
pub const PART_LEAF: u8 = 7;

/// How far a wall reaches below the lowest ground under its building (m), so none stands on air where the drawn land
/// lies a little below the heights.
pub const SINK_M: f32 = 1.5;
/// How deep a pier's sides reach under the sea (m).
pub const PIER_FOOT_M: f32 = -2.5;

/// Where a building's walls start (m above the sea): [`SINK_M`] below the lowest ground under it, or a part's base over
/// its mean ground ([`super::Part`]: nothing of it reaches down, nor sinks into the part it stands on).
pub fn foot_of(b: &Building) -> f32 {
    if b.base > 0.0 {
        b.ground[1] + b.base
    } else {
        b.ground[0] - SINK_M
    }
}

/// A vertex's style bytes.
pub fn style_of(wall: u8, roof: u8, seed: u8, kind: Kind, part: u8, detail: bool) -> [u8; 4] {
    [
        wall,
        roof,
        seed,
        (kind as u8 & 15) | ((part & 7) << 4) | (u8::from(detail) << 7),
    ]
}

/// A chunk's mesh: its vertices and triangles, its bounds (chunk-local x, z; heights above the sea) and the buildings in
/// it; its region ([`Region::index`], for the facades) and its roof clutter (whole chunks only, sorted by shape, and how
/// many of each).
#[derive(Clone, Debug, Default)]
pub struct ChunkMesh {
    pub vertices: Vec<BuildingVertex>,
    pub indices: Vec<u32>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub buildings: u32,
    /// Of `buildings`, the quays' cranes (whole chunks only).
    pub cranes: u32,
    pub region: u8,
    /// The age its buildings are drawn in.
    pub period: Period,
    pub clutter: Vec<Clutter>,
    pub clutter_counts: [u32; SHAPES],
    /// In a past period, the land its thinning kept whole whatever its score (a listed yard, a working waterfront, the
    /// ground about what a land's lift kept: [`thinning::Kept`]), for the period's ground; none today.
    pub kept: Option<thinning::Kept>,
    /// A far chunk's masses ([`Lod::Far`], [`super::far`]): no vertices, an instance each; its runs' first (this many),
    /// its blocks' after them.
    pub masses: Vec<Mass>,
    pub far_runs: u32,
}

impl ChunkMesh {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Bytes on the GPU (the clutter's too).
    pub fn bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<BuildingVertex>()
            + self.indices.len() * 4
            + self.clutter.len() * std::mem::size_of::<Clutter>()
            + self.masses.len() * std::mem::size_of::<Mass>()
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() && self.masses.is_empty()
    }
}

/// What [`build_chunk`] makes a chunk from.
pub struct ChunkInput<'a> {
    pub key: ChunkKey,
    /// The source's footprints whose middles lie in the chunk (OpenStreetMap's outlines merged in,
    /// `tags::merge_outlines`).
    pub footprints: &'a [Footprint],
    /// The infill's lots in the chunk (`infill::lots`).
    pub infill: &'a [Footprint],
    /// The map about the chunk (empty: the footprints typed by their shape alone).
    pub tags: &'a MapTags,
    /// The ground at a chunk-local (x, z), metres above the sea.
    pub ground: &'a dyn Fn([f32; 2]) -> f32,
    pub region: Region,
    /// The age the buildings are drawn in ([`Period`]).
    pub period: Period,
    /// What stood in a past period ([`Thinning`], the host's); none (or today) keeps every footprint.
    pub thinning: Option<&'a Thinning>,
}

/// A shop's big box on commercial land: the least footprint (m²) left out of the period outside an old core.
pub const SHOP_BOX_M2: f32 = 1500.0;

/// The least quay a crane stands on (m), and the least industrial land behind it (m²).
pub const CRANE_QUAY_M: f32 = 100.0;
pub const CRANE_LAND_M2: f32 = 20_000.0;

/// The chunk's mesh: the landmarks the host knows, every footprint planned ([`plan_building`], its setting from the
/// map, density from the source's footprints) and extruded, the piers' decks and the quays' cranes.
pub fn build_chunk(input: &ChunkInput) -> ChunkMesh {
    let density = DensityGrid::new(input.footprints);
    // A past period's Taiwan is made as East Asia (`Region::in_period`): its palettes, roofs, clutter and facades.
    let region = input.region.in_period(input.period);
    let mut mesh = MeshBuilder::default().with_region(region);
    mesh.mesh.period = input.period;
    let tags = input.tags;
    let ground = input.ground;
    let index = input.key.index;
    let lod = input.key.lod;
    let all: Vec<&Footprint> = input.footprints.iter().chain(input.infill).collect();
    let mut taken = vec![false; all.len()];
    // In an area the host draws itself: a footprint whose middle lies there (a part by its building's, all its parts
    // together), an infill lot reaching into it (the hole stays empty).
    let cleared = |k: usize, f: &Footprint| {
        !tags.cleared.is_empty()
            && if k < input.footprints.len() {
                tags.cleared_at(
                    f.part
                        .map_or_else(|| geometry::centroid(&f.ring), |p| p.middle),
                )
            } else {
                tags.cleared_over(&f.ring)
            }
    };
    let holding = |p: [f32; 2]| {
        all.iter().position(|f| {
            f.ring.len() >= 3
                && !f.raised()
                && (geometry::contains(&f.ring, p) || geometry::ring_distance(&f.ring, p) < 4.0)
        })
    };
    // The landmarks, and a lighthouse the map's points name where no footprint stands (none of the host's within 60 m):
    // each takes the footprint it stands on (but a keeper's house, which its tower rises from).
    // Nothing of the map's stands in an area the host draws itself (`MapTags::cleared`).
    let mut standing: Vec<Landmark> = tags.landmarks.clone();
    standing.retain(|l| !tags.cleared_at(l.at));
    for poi in &tags.pois {
        let at = poi.at;
        if poi.kind == PoiKind::Lighthouse
            && !tags.cleared_at(at)
            && (0.0..super::CHUNK_M as f32).contains(&at[0])
            && (0.0..super::CHUNK_M as f32).contains(&at[1])
            && holding(at).is_none()
            && !standing.iter().any(|l| geometry::distance(l.at, at) < 60.0)
        {
            standing.push(Landmark {
                at,
                kind: LandmarkKind::Lighthouse {
                    height: 16.0,
                    bands: 0,
                    red: true,
                    keeper: false,
                },
            });
        }
    }
    for landmark in &standing {
        let at = landmark.at;
        let under = holding(at);
        let seed = (hash01(
            i64::from(index[0]) * 1000 + at[0] as i64,
            i64::from(index[1]) * 1000 + at[1] as i64,
            61,
        ) * 255.0) as u8;
        let g = ground(at);
        // On a breakwater or a rock the heights put under the sea: a plinth to stand on.
        let footing = g < 0.5;
        let g = if footing { 2.5 } else { g };
        let mut b = Building {
            ring: geometry::regular(at, 2.0, 8, 0.0),
            kind: Kind::Lighthouse,
            roof: Roof::Flat,
            ground: [g, g, g],
            height: 3.0,
            base: 0.0,
            storeys: 1,
            storey_m: 3.0,
            wall: 13,
            roof_colour: 4,
            seed,
            detail: false,
            bay: 4.0,
            extras: Vec::new(),
            period: input.period,
        };
        match landmark.kind {
            LandmarkKind::Lighthouse {
                height,
                bands,
                red,
                keeper,
            } => {
                if keeper && let Some(k) = under {
                    taken[k] = true;
                    let mut house = all[k].clone();
                    house.height = Some(height);
                    let setting = super::Setting {
                        poi: Some(super::PoiKind::Lighthouse),
                        ..tags.setting(&house.ring, ground)
                    };
                    let c = geometry::centroid(&house.ring);
                    if let Some(mut planned) = plan_building(
                        &house,
                        index,
                        density.at(c),
                        region,
                        input.period,
                        &setting,
                        ground,
                    ) {
                        planned.extras.retain(|e| !matches!(e, Extra::Light { .. }));
                        planned.extras.push(Extra::Light {
                            centre: at,
                            radius: 1.8,
                            top: height.max(planned.height + planned.roof.rise() + 3.0),
                            bands,
                            red,
                        });
                        mesh.building(&planned, lod);
                    }
                    continue;
                }
                if let Some(k) = under {
                    taken[k] = true;
                }
                let radius = (height / 11.0).clamp(1.6, 4.0);
                b.extras.push(Extra::Light {
                    centre: at,
                    radius,
                    top: height - (1.6 + 0.06 * height) - 0.9 * radius * 0.6,
                    bands,
                    red,
                });
            }
            LandmarkKind::Chimney { height, radius } => {
                if let Some(k) = under {
                    taken[k] = true;
                }
                b.kind = Kind::Chimney;
                b.extras.push(Extra::Chimney {
                    centre: at,
                    radius,
                    top: height,
                    banded: height > 45.0,
                });
            }
        }
        if footing {
            mesh.plinth(&b, at);
        }
        mesh.building(&b, lod);
    }
    // A past period's thinning (never today's): only what stood, no taller than its place allowed; a listed yard and a
    // port's working waterfront whole, and the land they keep (with the ground about what a land's lift kept) handed to
    // the period's ground.
    let thin = input.thinning.filter(|_| input.period.is_past());
    let waterfront = thin.map(|t| thinning::Waterfront::new(tags, ground, t));
    let mut kept = thin.map(|_| thinning::Kept::default());
    // A whole chunk's houses' grounds, laid once every building stands (their neighbours' outlines known), and each
    // building's way to its road (its dormers' slope).
    let roads = Roads::new(&tags.roads);
    let mut footings = Footings::default();
    let mut yards: Vec<(Building, usize)> = Vec::new();
    // The buildings drawn by OSM's parts built after the period, by their middles (`Part::middle`): all of one goes
    // where any of its parts is listed, as its whole footprint went.
    let later_parts: Vec<[f32; 2]> = thin.map_or_else(Vec::new, |t| {
        input
            .footprints
            .iter()
            .filter(|f| f.ring.len() >= 3 && t.later(&f.ring))
            .filter_map(|f| f.part.map(|p| p.middle))
            .collect()
    });
    for (k, footprint) in all.iter().enumerate() {
        if taken[k] || footprint.ring.len() < 3 || cleared(k, footprint) {
            continue;
        }
        // A part is judged where its building stands (its parts kept or left out together).
        let c = footprint
            .part
            .map_or_else(|| geometry::centroid(&footprint.ring), |p| p.middle);
        let mut setting = tags.setting(&footprint.ring, ground);
        // Whether its ground is kept with it, once it stands (below).
        let mut its_ground = false;
        if let Some(t) = thin {
            if t.later(&footprint.ring)
                || footprint
                    .part
                    .is_some_and(|p| t.later_at(p.middle) || later_parts.contains(&p.middle))
            {
                continue;
            }
            let whole = t.in_yard(c) || waterfront.as_ref().is_some_and(|w| w.keeps(c, &setting));
            if !whole {
                let (worship, land) = thinning::lifts(tags, &setting, &footprint.ring, ground);
                if !t.keeps(c, worship + land) {
                    continue;
                }
                // Kept by its land's lift alone (railway or military land, a harbour's front): its ground with it.
                its_ground = land > 0.0 && !t.keeps(c, worship);
            } else {
                its_ground = true;
            }
            // A measured building over its place's cap stands at the cap (`plan_building`).
            setting.cap = Some(t.cap_for(&footprint.ring));
        }
        let Some(building) = plan_building(
            footprint,
            index,
            density.at(c),
            region,
            input.period,
            &setting,
            ground,
        ) else {
            continue;
        };
        // A shop's big box stands only in an old core (or a listed yard).
        if let Some(t) = thin
            && building.kind == Kind::Warehouse
            && setting.usage == Usage::Commercial
            && geometry::area(&footprint.ring) >= SHOP_BOX_M2
            && t.score_at(c) < thinning::CORE
            && !t.in_yard(c)
        {
            continue;
        }
        // A block or a row the source drew as one: the buildings it holds (`parcels`), each standing as its own; never one
        // of OSM's parts, drawn as mapped.
        let pieces = if footprint.part.is_some() {
            Vec::new()
        } else {
            parcels::split(
                footprint,
                &building,
                index,
                density.at(c),
                region,
                input.period,
                &setting,
                ground,
            )
        };
        let cut = !pieces.is_empty();
        let pieces = if cut { pieces } else { vec![building] };
        // Each piece's walls against its neighbours', drawn only above them.
        let mut parties = if cut {
            mesh.parties(&pieces, lod)
        } else {
            Vec::new()
        };
        for (i, building) in pieces.into_iter().enumerate() {
            if lod == Lod::Full {
                // Each piece faces the road nearest its own middle.
                let c = if cut {
                    geometry::centroid(&building.ring)
                } else {
                    c
                };
                mesh.front = roads.nearest(c, 40.0).and_then(|n| {
                    let d = [n.foot[0] - c[0], n.foot[1] - c[1]];
                    let l = d[0].hypot(d[1]);
                    (l > 1e-3).then(|| [d[0] / l, d[1] / l])
                });
            }
            mesh.set_party(parties.get_mut(i).map(std::mem::take).unwrap_or_default());
            mesh.building(&building, lod);
            mesh.front = None;
            mesh.party.clear();
            // A raised part's ground is its building's: no footing, no yard of its own.
            if lod == Lod::Full && building.base <= 0.0 {
                let me = footings.add(&building.ring);
                if matches!(
                    building.kind,
                    Kind::House | Kind::Terrace | Kind::Block | Kind::Tower
                ) {
                    yards.push((building, me));
                }
            }
        }
        if its_ground && let Some(kept) = kept.as_mut() {
            kept.set_ring(&footprint.ring, thinning::KEPT_MARGIN_M);
        }
    }
    for (b, me) in &yards {
        grounds::grounds(
            b,
            *me,
            region,
            &roads,
            &footings,
            ground,
            &mut mesh.mesh.clutter,
        );
    }
    // The piers whose middles lie in the chunk: their decks.
    for pier in &tags.piers {
        let c = geometry::centroid(&pier.ring);
        if (0.0..super::CHUNK_M as f32).contains(&c[0])
            && (0.0..super::CHUNK_M as f32).contains(&c[1])
            && !tags.cleared_at(c)
        {
            mesh.deck(pier, index);
        }
    }
    // Cranes along the quays of a port's large industrial waterfront.
    let mut quay_cranes = 0;
    if lod == Lod::Full && tags.port && !tags.zones.is_empty() {
        for quay in tags.quays(ground, CRANE_QUAY_M) {
            if quay.land_m2 < CRANE_LAND_M2 {
                continue;
            }
            for crane in cranes(&quay, index, input.period) {
                let Extra::Crane { at, .. } = crane else {
                    continue;
                };
                if tags.cleared_at(at)
                    || all
                        .iter()
                        .any(|f| f.ring.len() >= 3 && geometry::contains(&f.ring, at))
                {
                    continue;
                }
                let g = ground(at).max(1.5);
                let b = Building {
                    ring: geometry::regular(at, 4.0, 4, 0.0),
                    kind: Kind::Crane,
                    roof: Roof::Flat,
                    ground: [g, g, g],
                    height: 3.0,
                    base: 0.0,
                    storeys: 1,
                    storey_m: 3.0,
                    wall: 0,
                    roof_colour: 0,
                    seed: 0,
                    detail: false,
                    bay: 4.0,
                    extras: vec![crane],
                    period: input.period,
                };
                mesh.building(&b, lod);
                quay_cranes += 1;
            }
        }
    }
    if let (Some(t), Some(kept)) = (thin, kept.as_mut()) {
        let waterfront = waterfront.as_ref().filter(|w| w.active());
        if !t.yards.is_empty() || waterfront.is_some() {
            kept.set_where(|p| t.in_yard(p) || waterfront.is_some_and(|w| w.land(p)));
        }
    }
    let mut out = mesh.finish();
    out.cranes = quay_cranes;
    out.kept = kept;
    out
}

/// The cranes along a quay: every 70-130 m (by a hash of the quay's place) 12 m back from its edge, their jibs out over
/// the water; container gantries along a long quay before a large yard (today: in a past `period` jib cranes in their
/// place, painted dark), jib cranes elsewhere; at most eight.
pub fn cranes(quay: &Quay, index: [i32; 2], period: Period) -> Vec<Extra> {
    let len = quay.length();
    let key = |p: [f32; 2], k: i64| {
        hash01(
            i64::from(index[0]) * 100_000 + (p[0] * 0.1) as i64,
            i64::from(index[1]) * 100_000 + (p[1] * 0.1) as i64,
            k,
        )
    };
    let gantry =
        !period.is_past() && len >= 300.0 && quay.land_m2 >= 100_000.0 && key(quay.a, 71) < 0.6;
    let spacing = if gantry {
        70.0 + 30.0 * key(quay.a, 72)
    } else {
        80.0 + 50.0 * key(quay.a, 72)
    };
    let colours: &[[u8; 3]] = if gantry {
        &[
            [204, 51, 51],
            [51, 102, 153],
            [255, 153, 51],
            [204, 204, 204],
        ]
    } else if period.is_past() {
        // The period's cranes: black, grey or a dull red lead.
        &[
            [51, 51, 51],
            [102, 102, 102],
            [153, 153, 153],
            [128, 64, 51],
        ]
    } else {
        &[
            [255, 204, 0],
            [51, 51, 51],
            [204, 51, 51],
            [102, 153, 102],
            [153, 153, 153],
        ]
    };
    let colour = super::cube_entry(
        colours[((key(quay.a, 73) * colours.len() as f32) as usize).min(colours.len() - 1)],
    );
    let dir = [
        (quay.b[0] - quay.a[0]) / len.max(1e-3),
        (quay.b[1] - quay.a[1]) / len.max(1e-3),
    ];
    let mut out = Vec::new();
    let mut s = 20.0 + key(quay.b, 74) * (spacing - 20.0);
    while s < len - 15.0 && out.len() < 8 {
        let p = [quay.a[0] + dir[0] * s, quay.a[1] + dir[1] * s];
        out.push(Extra::Crane {
            at: [p[0] - quay.out[0] * 12.0, p[1] - quay.out[1] * 12.0],
            out: quay.out,
            gantry,
            colour,
        });
        s += spacing;
    }
    out
}

/// Accumulates buildings into a [`ChunkMesh`]; with a region, their roof clutter too (whole buildings only).
#[derive(Default)]
pub struct MeshBuilder {
    mesh: ChunkMesh,
    clutter: Option<Region>,
    /// The next building's front: the way to its road (chunk-local x, z), for its dormers.
    front: Option<[f32; 2]>,
    /// The next building's walls against its neighbours' (a block's pieces, `parcels`): drawn only above theirs.
    party: Vec<Party>,
}

/// A stretch of a building's wall against a neighbour's (a block's pieces back to back, `parcels`): from `from` to `to`
/// along the building's own edge (chunk-local x, z), hidden up to `top`, the top of the neighbour's walls (metres above
/// the sea).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Party {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub top: f32,
}

/// How far (m) an edge may lie off its neighbour's and still be one wall with it.
const PARTY_M: f32 = 0.05;

/// A wall's facade: its storeys counted from `ground`, `rows` of windows, bays about `bay` wide, in `style`.
struct Wall {
    ground: f32,
    storey_m: f32,
    rows: i8,
    bay: f32,
    style: [u8; 4],
}

fn f16_bits(v: f32) -> u16 {
    f16::from_f32(v).to_bits()
}

fn snorm(n: [f32; 3], w: i8) -> [i8; 4] {
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
    [q(n[0]), q(n[1]), q(n[2]), w]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

impl MeshBuilder {
    fn style(b: &Building, part: u8) -> [u8; 4] {
        style_of(b.wall, b.roof_colour, b.seed, b.kind, part, b.detail)
    }

    pub(crate) fn vertex_count(&self) -> u32 {
        self.mesh.vertices.len() as u32
    }

    pub(crate) fn push_triangle(&mut self, t: [u32; 3]) {
        self.mesh.indices.extend_from_slice(&t);
    }

    /// A planar polygon face (its points in order, counter-clockwise seen from `outward`'s side), fanned from its first
    /// point: one vertex each with `facade`'s coordinates.
    pub(crate) fn face(
        &mut self,
        points: &[[f32; 3]],
        facade: &[[f32; 2]],
        outward: [f32; 3],
        rows: i8,
        style: [u8; 4],
    ) {
        if points.len() < 3 {
            return;
        }
        let first = self.mesh.vertices.len() as u32;
        // The face's own normal, turned to face `outward`.
        let mut n = normalize(cross(sub(points[1], points[0]), sub(points[2], points[0])));
        let flip = n[0] * outward[0] + n[1] * outward[1] + n[2] * outward[2] < 0.0;
        if flip || n.iter().all(|c| c.abs() < 1e-6) {
            n = normalize(outward);
        }
        for (p, f) in points.iter().zip(facade) {
            self.push_vertex(*p, n, *f, rows, style);
        }
        for k in 1..points.len() as u32 - 1 {
            if flip {
                self.mesh
                    .indices
                    .extend_from_slice(&[first, first + k + 1, first + k]);
            } else {
                self.mesh
                    .indices
                    .extend_from_slice(&[first, first + k, first + k + 1]);
            }
        }
    }

    pub(crate) fn push_vertex(
        &mut self,
        p: [f32; 3],
        n: [f32; 3],
        facade: [f32; 2],
        rows: i8,
        style: [u8; 4],
    ) {
        let m = &mut self.mesh;
        if m.vertices.is_empty() {
            m.min = p;
            m.max = p;
        }
        for (k, v) in p.iter().enumerate() {
            m.min[k] = m.min[k].min(*v);
            m.max[k] = m.max[k].max(*v);
        }
        m.vertices.push(BuildingVertex {
            position: p,
            normal: snorm(n, rows),
            facade: [f16_bits(facade[0]), f16_bits(facade[1])],
            style,
        });
    }

    /// The walls of `ring` from `bottom` to `top` (heights above the sea), storeys of `storey_m` counted from `ground`,
    /// `rows` of windows, bays about `bay` wide.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn walls_with(
        &mut self,
        ring: &[[f32; 2]],
        bottom: f32,
        top: f32,
        ground: f32,
        storey_m: f32,
        rows: i8,
        bay: f32,
        style: [u8; 4],
    ) {
        let wall = Wall {
            ground,
            storey_m,
            rows,
            bay,
            style,
        };
        let n = ring.len();
        for i in 0..n {
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            self.wall_span(a, c, [0.0, 1.0], bottom, top, &wall);
        }
    }

    /// The walls of `ring` from `bottom` to `top`, storeys counted from the building's mean ground.
    fn walls(&mut self, b: &Building, ring: &[[f32; 2]], bottom: f32, top: f32, part: u8) {
        let rows = b.storeys.min(127) as i8;
        let style = Self::style(b, part);
        if part != PART_WALL || self.party.is_empty() {
            self.walls_with(
                ring,
                bottom,
                top,
                b.ground[1],
                b.storey_m,
                rows,
                b.bay,
                style,
            );
            return;
        }
        // Against a neighbour's walls, only what stands above them: the rest is inside the neighbour (and a lower
        // neighbour's parapet would fight the wall above its roof for the same pixels).
        let wall = Wall {
            ground: b.ground[1],
            storey_m: b.storey_m,
            rows,
            bay: b.bay,
            style,
        };
        let n = ring.len();
        for i in 0..n {
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            for (along, lo) in self.wall_spans(a, c, bottom, top) {
                self.wall_span(a, c, along, lo, top, &wall);
            }
        }
    }

    /// The stretches of the wall from `a` to `c` (as fractions along it) and each one's foot: `bottom`, or the top of
    /// the neighbour's walls it stands against ([`Party`]); none where the neighbour stands as high as `top`.
    fn wall_spans(&self, a: [f32; 2], c: [f32; 2], bottom: f32, top: f32) -> Vec<([f32; 2], f32)> {
        let len = geometry::distance(a, c);
        if len < 0.05 {
            return Vec::new();
        }
        let d = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
        let off = |x: [f32; 2]| (x[0] - a[0]) * d[1] - (x[1] - a[1]) * d[0];
        let at = |x: [f32; 2]| ((x[0] - a[0]) * d[0] + (x[1] - a[1]) * d[1]) / len;
        let against: Vec<(f32, f32, f32)> = self
            .party
            .iter()
            .filter(|p| off(p.from).abs() <= PARTY_M && off(p.to).abs() <= PARTY_M)
            .filter_map(|p| {
                let (t0, t1) = (at(p.from), at(p.to));
                let (t0, t1) = (t0.min(t1).max(0.0), t0.max(t1).min(1.0));
                (t1 - t0 > 1e-3).then_some((t0, t1, p.top))
            })
            .collect();
        if against.is_empty() {
            return vec![([0.0, 1.0], bottom)];
        }
        let mut cuts: Vec<f32> = vec![0.0, 1.0];
        cuts.extend(against.iter().flat_map(|&(t0, t1, _)| [t0, t1]));
        cuts.sort_by(f32::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() < 1e-4);
        let mut spans: Vec<([f32; 2], f32)> = Vec::new();
        for w in cuts.windows(2) {
            let mid = 0.5 * (w[0] + w[1]);
            let cover = against
                .iter()
                .filter(|&&(t0, t1, _)| t0 <= mid && mid <= t1)
                .map(|&(_, _, top)| top)
                .fold(f32::NEG_INFINITY, f32::max);
            let lo = bottom.max(cover);
            if lo >= top - 0.01 {
                continue;
            }
            match spans.last_mut() {
                Some((along, foot)) if *foot == lo && (along[1] - w[0]).abs() < 1e-4 => {
                    along[1] = w[1]
                }
                _ => spans.push(([w[0], w[1]], lo)),
            }
        }
        spans
    }

    /// The wall from `a` to `c` over the stretch `along` (fractions of it), from `lo` to `hi`: its bays counted over
    /// the whole edge, so a stretch's windows stand where the whole wall's would.
    fn wall_span(
        &mut self,
        a: [f32; 2],
        c: [f32; 2],
        along: [f32; 2],
        lo: f32,
        hi: f32,
        wall: &Wall,
    ) {
        let len = geometry::distance(a, c);
        if len < 0.05 {
            return;
        }
        let storey = |y: f32| (y - wall.ground) / wall.storey_m;
        let bays = (len / wall.bay.max(0.5)).round().max(1.0);
        let outward = [(c[1] - a[1]) / len, 0.0, -(c[0] - a[0]) / len];
        let p = |t: f32| [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t];
        let (p0, p1) = (p(along[0]), p(along[1]));
        self.face(
            &[
                [p0[0], lo, p0[1]],
                [p0[0], hi, p0[1]],
                [p1[0], hi, p1[1]],
                [p1[0], lo, p1[1]],
            ],
            &[
                [bays * along[0], storey(lo)],
                [bays * along[0], storey(hi)],
                [bays * along[1], storey(hi)],
                [bays * along[1], storey(lo)],
            ],
            outward,
            wall.rows,
            wall.style,
        );
    }

    /// The top of `b`'s walls at `lod` (metres above the sea): its eaves, a flat roof's parapet over them, and on a box
    /// a pitched roof's half rise.
    fn wall_top(&self, b: &Building, lod: Lod) -> f32 {
        let eave = (b.ground[1] + b.height).max(b.ground[2] + 2.2);
        let parapet = self.clutter.map_or(0.0, |r| parapet_of(b, r));
        match lod {
            Lod::Full => eave + parapet,
            Lod::Box | Lod::Far => eave + 0.5 * b.roof.rise() + parapet,
        }
    }

    /// Each of `pieces`' (one footprint's buildings, `parcels`) stretches of wall against another's at `lod`: where an
    /// edge of one runs along an edge of another, back to back (both counter-clockwise, so the other way), with the
    /// other's walls' top.
    pub(crate) fn parties(&self, pieces: &[Building], lod: Lod) -> Vec<Vec<Party>> {
        let tops: Vec<f32> = pieces.iter().map(|b| self.wall_top(b, lod)).collect();
        let boxes: Vec<[f32; 4]> = pieces
            .iter()
            .map(|b| {
                b.ring.iter().fold(
                    [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
                    |[x0, z0, x1, z1], p| [x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1])],
                )
            })
            .collect();
        let touch = |i: usize, j: usize| {
            let (a, b) = (boxes[i], boxes[j]);
            a[0] <= b[2] + PARTY_M
                && b[0] <= a[2] + PARTY_M
                && a[1] <= b[3] + PARTY_M
                && b[1] <= a[3] + PARTY_M
        };
        let mut out = vec![Vec::new(); pieces.len()];
        for (i, one) in pieces.iter().enumerate() {
            for (j, other) in pieces.iter().enumerate() {
                if i == j || !touch(i, j) {
                    continue;
                }
                let n = one.ring.len();
                for k in 0..n {
                    let (a, c) = (one.ring[k], one.ring[(k + 1) % n]);
                    let len = geometry::distance(a, c);
                    if len < 0.1 {
                        continue;
                    }
                    let d = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
                    let off = |x: [f32; 2]| (x[0] - a[0]) * d[1] - (x[1] - a[1]) * d[0];
                    let at = |x: [f32; 2]| (x[0] - a[0]) * d[0] + (x[1] - a[1]) * d[1];
                    let m = other.ring.len();
                    for e in 0..m {
                        let (r, q) = (other.ring[e], other.ring[(e + 1) % m]);
                        if off(r).abs() > PARTY_M || off(q).abs() > PARTY_M {
                            continue;
                        }
                        // Back to back: the other's edge runs the other way along the line.
                        let (sr, sq) = (at(r), at(q));
                        if sq >= sr {
                            continue;
                        }
                        let (lo, hi) = (sq.max(0.0), sr.min(len));
                        if hi - lo < 0.1 {
                            continue;
                        }
                        out[i].push(Party {
                            from: [a[0] + d[0] * lo, a[1] + d[1] * lo],
                            to: [a[0] + d[0] * hi, a[1] + d[1] * hi],
                            top: tops[j],
                        });
                    }
                }
            }
        }
        out
    }

    /// Hands the next building its walls against its neighbours' ([`Party`]); cleared once it is built.
    pub(crate) fn set_party(&mut self, party: Vec<Party>) {
        self.party = party;
    }

    /// A flat roof over `ring` at `y` in `style`.
    pub(crate) fn flat_ring(&mut self, ring: &[[f32; 2]], y: f32, style: [u8; 4]) {
        let first = self.mesh.vertices.len() as u32;
        let c = geometry::centroid(ring);
        for p in ring {
            self.push_vertex(
                [p[0], y, p[1]],
                [0.0, 1.0, 0.0],
                [p[0] - c[0], p[1] - c[1]],
                0,
                style,
            );
        }
        // Counter-clockwise in (x, z) faces down: each triangle reversed.
        for t in geometry::triangulate(ring) {
            self.mesh
                .indices
                .extend_from_slice(&[first + t[0], first + t[2], first + t[1]]);
        }
    }

    /// A flat roof over `ring` at `y`.
    fn flat_roof(&mut self, b: &Building, ring: &[[f32; 2]], y: f32) {
        self.flat_ring(ring, y, Self::style(b, PART_ROOF));
    }

    /// A raised part's underside over `ring` at its foot `y` ([`foot_of`]), facing down and plain in the walls' colour:
    /// a skybridge's floor seen from the street, a crown's overhang over its shaft (where it rests on a roof, the two
    /// face away from each other and only one is ever drawn; the shadows' casters draw both faces).
    fn soffit(&mut self, b: &Building, ring: &[[f32; 2]], y: f32) {
        let first = self.mesh.vertices.len() as u32;
        let c = geometry::centroid(ring);
        let style = Self::style(b, PART_GABLE);
        for p in ring {
            self.push_vertex(
                [p[0], y, p[1]],
                [0.0, -1.0, 0.0],
                [p[0] - c[0], p[1] - c[1]],
                0,
                style,
            );
        }
        // Counter-clockwise in (x, z) faces down as it is (`flat_ring`'s reversed).
        for t in geometry::triangulate(ring) {
            self.mesh
                .indices
                .extend_from_slice(&[first + t[0], first + t[1], first + t[2]]);
        }
    }

    /// A pitched roof over the four corners `r` (a rectangle, counter-clockwise, its ridge's side first) from the eaves
    /// at `eave`, rising `rise`; hipped or gabled. A gable's triangles are walls. With `eaves` (a whole building's in its
    /// region, [`eaves_of`]) the roof is a slab its thickness deep at the walls held out its overhang past them (level
    /// all round a hipped roof): its soffits under the overhang, its fascia along the eaves with the gutter's drop below
    /// it, its ends over the gables, a downpipe at two corners; and `dormers` on the slope facing the building's front.
    #[allow(clippy::too_many_arguments)]
    fn pitched_roof(
        &mut self,
        b: &Building,
        r: &[[f32; 2]],
        eave: f32,
        rise: f32,
        hipped: bool,
        eaves: Option<Eaves>,
        dormers: u32,
    ) {
        let [p0, p1, p2, p3] = [r[0], r[1], r[2], r[3]];
        let long = geometry::distance(p0, p1);
        let short = geometry::distance(p1, p2);
        let half = short * 0.5;
        let mid = |a: [f32; 2], c: [f32; 2]| [(a[0] + c[0]) * 0.5, (a[1] + c[1]) * 0.5];
        let axis = [(p1[0] - p0[0]) / long, (p1[1] - p0[1]) / long];
        // Out of the p0-p1 side (the ring counter-clockwise).
        let out = [axis[1], -axis[0]];
        // The ridge from the p3-p0 end to the p1-p2 end, drawn in by half the short side on a hipped roof.
        let inset = if hipped {
            (short * 0.5).min(long * 0.5)
        } else {
            0.0
        };
        let (mut ra, mut rb) = (mid(p3, p0), mid(p1, p2));
        ra = [ra[0] + axis[0] * inset, ra[1] + axis[1] * inset];
        rb = [rb[0] - axis[0] * inset, rb[1] - axis[1] * inset];
        let (o, t, drop) = eaves.map_or((0.0, 0.0, 0.0), |e| (e.overhang, e.thickness, e.gutter));
        let tan = rise / half.max(1e-3);
        // A hipped roof's ends held out as far below the walls' tops as its sides, so its eaves run level all round.
        let oa = if hipped && inset > 1e-3 {
            o * inset / half.max(1e-3)
        } else {
            0.0
        };
        let grow = |p: [f32; 2], side: f32, end: f32| {
            [
                p[0] + out[0] * o * side + axis[0] * oa * end,
                p[1] + out[1] * o * side + axis[1] * oa * end,
            ]
        };
        let (q0, q1, q2, q3) = (
            grow(p0, 1.0, -1.0),
            grow(p1, 1.0, 1.0),
            grow(p2, -1.0, 1.0),
            grow(p3, -1.0, -1.0),
        );
        // The slab's top at its outer edge and its underside there; the ridge (its underside at the gables' apex).
        let lip = eave + t - o * tan;
        let under = eave - o * tan;
        let apex = eave + rise;
        let top = apex + t;
        let at = |p: [f32; 2], y: f32| [p[0], y, p[1]];
        let roof = Self::style(b, PART_ROOF);
        let slope = ((rise + o * tan).powi(2) + (half + o).powi(2)).sqrt();
        // Facade on a slope: metres down from the ridge, metres along it.
        let run = |p: [f32; 2]| (p[0] - p0[0]) * axis[0] + (p[1] - p0[1]) * axis[1];
        let normal_of = |a: [f32; 2], c: [f32; 2], up: f32| {
            let d = [c[0] - a[0], c[1] - a[1]];
            let l = d[0].hypot(d[1]).max(1e-6);
            normalize([d[1] / l * rise, up, -d[0] / l * rise])
        };
        // The two long slopes: p0-p1 under ra-rb, p2-p3 under rb-ra.
        let o1 = normal_of(p0, p1, half);
        self.face(
            &[at(q0, lip), at(ra, top), at(rb, top), at(q1, lip)],
            &[
                [slope, run(q0)],
                [0.0, run(ra)],
                [0.0, run(rb)],
                [slope, run(q1)],
            ],
            o1,
            0,
            roof,
        );
        let o2 = normal_of(p2, p3, half);
        self.face(
            &[at(q2, lip), at(rb, top), at(ra, top), at(q3, lip)],
            &[
                [slope, run(q2)],
                [0.0, run(rb)],
                [0.0, run(ra)],
                [slope, run(q3)],
            ],
            o2,
            0,
            roof,
        );
        let (h1, h2) = (
            normal_of(p1, p2, inset.max(0.01)),
            normal_of(p3, p0, inset.max(0.01)),
        );
        if hipped {
            // The hips: triangles over the short ends.
            let end = ((inset + oa).powi(2) + (rise + o * tan).powi(2)).sqrt();
            let across = short + 2.0 * o;
            for (a, c, ridge, n) in [(q1, q2, rb, h1), (q3, q0, ra, h2)] {
                self.face(
                    &[at(a, lip), at(ridge, top), at(c, lip)],
                    &[[end, 0.0], [0.0, 0.5 * across], [end, across]],
                    n,
                    0,
                    roof,
                );
            }
        } else {
            // The gables: wall triangles, storeys counted on as the wall's.
            let style = Self::style(b, PART_GABLE);
            let storey = |y: f32| (y - b.ground[1]) / b.storey_m;
            let bays = (short / b.bay.max(0.5)).round().max(1.0);
            for (a, c, ridge) in [(p1, p2, rb), (p3, p0, ra)] {
                let len = geometry::distance(a, c).max(1e-6);
                let outward = [(c[1] - a[1]) / len, 0.0, -(c[0] - a[0]) / len];
                self.face(
                    &[at(a, eave), at(ridge, apex), at(c, eave)],
                    &[
                        [0.0, storey(eave)],
                        [0.5 * bays, storey(apex)],
                        [bays, storey(eave)],
                    ],
                    outward,
                    b.storeys.min(127) as i8,
                    style,
                );
            }
        }
        let Some(e) = eaves else {
            return;
        };
        let fascia = style_of(e.fascia, b.roof_colour, b.seed, b.kind, PART_STEEL, false);
        let soffit = style_of(e.soffit, b.roof_colour, b.seed, b.kind, PART_STEEL, false);
        let flat = [[0.0, 0.0]; 4];
        // Each run of eaves: its wall's corners, the slab's edge over them, the slope's normal.
        let mut runs = vec![(p0, p1, q0, q1, o1), (p2, p3, q2, q3, o2)];
        if hipped {
            runs.extend([(p1, p2, q1, q2, h1), (p3, p0, q3, q0, h2)]);
        }
        for (a, c, qa, qc, n) in runs {
            // The soffit: the slab's underside over the overhang, facing down and out.
            self.face(
                &[at(a, eave), at(c, eave), at(qc, under), at(qa, under)],
                &flat,
                [-n[0], -n[1], -n[2]],
                0,
                soffit,
            );
            // The fascia, the gutter's face below it.
            let len = geometry::distance(qa, qc).max(1e-6);
            let outward = [(qc[1] - qa[1]) / len, 0.0, -(qc[0] - qa[0]) / len];
            self.face(
                &[
                    at(qa, under - drop),
                    at(qa, lip),
                    at(qc, lip),
                    at(qc, under - drop),
                ],
                &flat,
                outward,
                0,
                fascia,
            );
        }
        if !hipped && t > 0.0 {
            // The slab's ends over the gables (a verge flush with them).
            for (qa, qc, ridge, sign) in [(q1, q2, rb, 1.0), (q3, q0, ra, -1.0)] {
                let outward = [axis[0] * sign, 0.0, axis[1] * sign];
                self.face(
                    &[at(qa, under), at(qa, lip), at(ridge, top), at(ridge, apex)],
                    &flat,
                    outward,
                    0,
                    fascia,
                );
                self.face(
                    &[at(ridge, apex), at(ridge, top), at(qc, lip), at(qc, under)],
                    &flat,
                    outward,
                    0,
                    fascia,
                );
            }
        }
        // A downpipe from each long run's gutter, at diagonal corners, standing off the wall under it.
        if drop > 0.0 && self.clutter.is_some() {
            let foot = b.ground[0] - 0.2;
            let pipe = style_of(e.fascia, b.roof_colour, b.seed, b.kind, PART_STEEL, false);
            let yaw = axis[1].atan2(axis[0]);
            for (corner, side, end) in [(p1, 1.0, -1.0), (p3, -1.0, 1.0)] {
                let p = [
                    corner[0] + out[0] * 0.07 * side + axis[0] * 0.15 * end,
                    corner[1] + out[1] * 0.07 * side + axis[1] * 0.15 * end,
                ];
                self.mesh.clutter.push(Clutter {
                    at: [p[0], foot, p[1]],
                    yaw,
                    size: [0.09, (under - drop - foot).max(0.5), 0.09],
                    shape: Shape::Near as u32,
                    style: pipe,
                });
            }
        }
        if dormers == 0 {
            return;
        }
        // The dormers: on the slope facing the building's front (its road), else either.
        let front = self.front.map_or(place_hash(b, 23) < 0.5, |f| {
            f[0] * out[0] + f[1] * out[1] >= 0.0
        });
        let (pa, n_out) = if front {
            (p0, out)
        } else {
            (p2, [-out[0], -out[1]])
        };
        let u = if front { axis } else { [-axis[0], -axis[1]] };
        let usable = if hipped { long - 2.0 * inset } else { long } - 1.2;
        let n = dormers.min((usable / 2.6).floor().max(0.0) as u32);
        if n == 0 {
            return;
        }
        let width = (1.35 + 0.3 * place_hash(b, 24)).min(usable / n as f32 - 0.8);
        let depth = 0.5 + 0.3 * place_hash(b, 25);
        let tall = 1.5;
        let y_b = eave + t + depth * tan;
        let y_e = y_b + tall;
        let y_r = y_e + width * 0.5 * tan;
        let d_e = depth + tall / tan;
        let d_r = d_e + width * 0.5;
        // Its ridge under the roof's.
        if width < 1.0 || d_r > half - 0.3 {
            return;
        }
        let spacing = usable / n as f32;
        let centre = long * 0.5;
        let place = |s: f32, d: f32| {
            [
                pa[0] + u[0] * s - n_out[0] * d,
                pa[1] + u[1] * s - n_out[1] * d,
            ]
        };
        let rows = (b.storeys + 1).min(127) as i8;
        let wall = Self::style(b, PART_WALL);
        let cheek = Self::style(b, PART_GABLE);
        // One bay of the attic's storey, its sill a little over the slope: a window.
        let attic = f32::from(b.storeys) + 0.27;
        let storey = |y: f32| attic + (y - y_b) / b.storey_m;
        let pitch = ((width * 0.5).powi(2) + (y_r - y_e).powi(2)).sqrt();
        for k in 0..n {
            let s = centre + (k as f32 - (n as f32 - 1.0) * 0.5) * spacing;
            let (lf, rf, af) = (
                place(s - width * 0.5, depth),
                place(s + width * 0.5, depth),
                place(s, depth),
            );
            let (lb, rb, ab) = (
                place(s - width * 0.5, d_e),
                place(s + width * 0.5, d_e),
                place(s, d_r),
            );
            self.face(
                &[
                    at(lf, y_b),
                    at(rf, y_b),
                    at(rf, y_e),
                    at(af, y_r),
                    at(lf, y_e),
                ],
                &[
                    [0.0, storey(y_b)],
                    [1.0, storey(y_b)],
                    [1.0, storey(y_e)],
                    [0.5, storey(y_r)],
                    [0.0, storey(y_e)],
                ],
                [n_out[0], 0.0, n_out[1]],
                rows,
                wall,
            );
            // Its cheeks, back to the slope.
            self.face(
                &[at(lf, y_b), at(lf, y_e), at(lb, y_e)],
                &[[0.0, 0.0]; 3],
                [-u[0], 0.0, -u[1]],
                0,
                cheek,
            );
            self.face(
                &[at(rf, y_b), at(rb, y_e), at(rf, y_e)],
                &[[0.0, 0.0]; 3],
                [u[0], 0.0, u[1]],
                0,
                cheek,
            );
            // Its roof, running back into the slope.
            let back = d_e - depth;
            let ridge_back = d_r - depth;
            for (side, front_eave, back_eave) in [(-1.0, lf, lb), (1.0, rf, rb)] {
                let n = normalize([
                    u[0] * side * (y_r - y_e),
                    width * 0.5,
                    u[1] * side * (y_r - y_e),
                ]);
                self.face(
                    &[
                        at(front_eave, y_e),
                        at(af, y_r),
                        at(ab, y_r),
                        at(back_eave, y_e),
                    ],
                    &[[pitch, 0.0], [0.0, 0.0], [0.0, ridge_back], [pitch, back]],
                    n,
                    0,
                    roof,
                );
            }
        }
    }

    /// A flat roof's parapet over `ring` from `y` up `height`: its inner face (the walls carry its outer one).
    fn parapet(&mut self, b: &Building, ring: &[[f32; 2]], y: f32, height: f32) {
        let style = Self::style(b, PART_GABLE);
        let storey = |h: f32| (h - b.ground[1]) / b.storey_m;
        let n = ring.len();
        for i in 0..n {
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            let len = geometry::distance(a, c);
            if len < 0.05 {
                continue;
            }
            let bays = (len / b.bay.max(0.5)).round().max(1.0);
            let inward = [-(c[1] - a[1]) / len, 0.0, (c[0] - a[0]) / len];
            self.face(
                &[
                    [c[0], y, c[1]],
                    [c[0], y + height, c[1]],
                    [a[0], y + height, a[1]],
                    [a[0], y, a[1]],
                ],
                &[
                    [0.0, storey(y)],
                    [0.0, storey(y + height)],
                    [bays, storey(y + height)],
                    [bays, storey(y)],
                ],
                inward,
                0,
                style,
            );
        }
    }

    /// A sawtooth roof over the four corners `r` (long side first): teeth across the long side, each a slope rising
    /// `rise` over `tooth` metres to a glazed upright face, its ends walls.
    fn sawtooth_roof(&mut self, b: &Building, r: &[[f32; 2]], eave: f32, rise: f32, tooth: f32) {
        let [p0, p1, _p2, p3] = [r[0], r[1], r[2], r[3]];
        let long = geometry::distance(p0, p1);
        let teeth = (long / tooth.max(2.0)).round().max(1.0) as usize;
        let along =
            |t: f32, side: [f32; 2]| [side[0] + (p1[0] - p0[0]) * t, side[1] + (p1[1] - p0[1]) * t];
        let axis = [(p1[0] - p0[0]) / long, 0.0, (p1[1] - p0[1]) / long];
        let width = geometry::distance(p0, p3);
        let step = long / teeth as f32;
        let slope = (step * step + rise * rise).sqrt();
        let roof = Self::style(b, PART_ROOF);
        let glass = style_of(b.wall, b.roof_colour, b.seed, b.kind, PART_GLASS, false);
        let gable = Self::style(b, PART_GABLE);
        // Out of the long side p0-p1 (the ring counter-clockwise): the ends' triangles stand in the long walls' planes.
        let side_out = [axis[2], 0.0, -axis[0]];
        let up = |p: [f32; 2], y: f32| [p[0], y, p[1]];
        for k in 0..teeth {
            let (t0, t1) = (k as f32 / teeth as f32, (k + 1) as f32 / teeth as f32);
            let (a0, a1) = (along(t0, p0), along(t1, p0));
            let (b0, b1) = (along(t0, p3), along(t1, p3));
            // The slope rising toward +axis.
            let n = normalize([-axis[0] * rise, step, -axis[2] * rise]);
            self.face(
                &[
                    up(a0, eave),
                    up(b0, eave),
                    up(b1, eave + rise),
                    up(a1, eave + rise),
                ],
                &[[slope, 0.0], [slope, width], [0.0, width], [0.0, 0.0]],
                n,
                0,
                roof,
            );
            // The glazed face looking along +axis.
            self.face(
                &[
                    up(a1, eave),
                    up(a1, eave + rise),
                    up(b1, eave + rise),
                    up(b1, eave),
                ],
                &[
                    [0.0, 0.0],
                    [0.0, 1.0],
                    [width / 3.0, 1.0],
                    [width / 3.0, 0.0],
                ],
                axis,
                1,
                glass,
            );
            // The tooth's two end triangles.
            self.face(
                &[up(a0, eave), up(a1, eave + rise), up(a1, eave)],
                &[[0.0, 0.0]; 3],
                side_out,
                0,
                gable,
            );
            self.face(
                &[up(b0, eave), up(b1, eave), up(b1, eave + rise)],
                &[[0.0, 0.0]; 3],
                [-side_out[0], 0.0, -side_out[2]],
                0,
                gable,
            );
        }
    }

    /// A pier's deck: its edges from under the sea to the deck, its deck on top.
    pub(crate) fn deck(&mut self, pier: &Pier, index: [i32; 2]) {
        let c = geometry::centroid(&pier.ring);
        let h = |k: i64| {
            hash01(
                i64::from(index[0]) * 10_000 + c[0] as i64,
                i64::from(index[1]) * 10_000 + c[1] as i64,
                k,
            )
        };
        let wall = super::pick_entry(&[12, 3, 2, 12, 30], (h(81) * 255.0) as u8);
        let roof = super::pick_entry(&[14, 9, 13, 14, 22], (h(82) * 255.0) as u8);
        let seed = (h(83) * 255.0) as u8;
        let deck = pier.deck_m();
        let side = style_of(wall, roof, seed, Kind::Deck, PART_GABLE, false);
        let top = style_of(wall, roof, seed, Kind::Deck, PART_ROOF, false);
        self.walls_with(&pier.ring, PIER_FOOT_M, deck, 0.0, 3.0, 0, 4.0, side);
        self.flat_ring(&pier.ring, deck, top);
        self.mesh.buildings += 1;
    }

    /// A plinth under a landmark the heights put under the sea (a lighthouse on a breakwater's head): a squat drum from
    /// under the water to its ground.
    pub(crate) fn plinth(&mut self, b: &Building, at: [f32; 2]) {
        let style = style_of(3, 14, b.seed, Kind::Deck, PART_GABLE, false);
        let top = style_of(3, 14, b.seed, Kind::Deck, PART_ROOF, false);
        let plain = |y: f32| y;
        let facade = super::parts::Facade {
            around: 8.0,
            rows_at: &plain,
            rows: 0,
        };
        self.prism(
            at,
            6.0,
            5.5,
            PIER_FOOT_M,
            b.ground[1],
            10,
            0.0,
            false,
            &facade,
            style,
        );
        self.disc(at, 5.5, b.ground[1], 10, 0.0, false, top);
    }

    /// One building at `lod`.
    pub fn building(&mut self, b: &Building, lod: Lod) {
        let ring = &b.ring;
        if ring.len() < 3 || signed_area2(ring) <= 0.0 {
            return;
        }
        let bottom = foot_of(b);
        // The eaves over the mean ground, and never under the highest ground's storey.
        let eave = (b.ground[1] + b.height).max(b.ground[2] + 2.2);
        match b.kind {
            // Drawn by their extras alone.
            Kind::Lighthouse | Kind::Chimney | Kind::Crane => {}
            Kind::Tank => {
                let c = geometry::centroid(ring);
                let r = ring
                    .iter()
                    .map(|p| geometry::distance(*p, c))
                    .fold(0.0, f32::max);
                let plain = |y: f32| y;
                let facade = super::parts::Facade {
                    around: 12.0,
                    rows_at: &plain,
                    rows: 0,
                };
                let wall = Self::style(b, PART_GABLE);
                let roof = Self::style(b, PART_ROOF);
                let (sides, smooth) = match lod {
                    Lod::Full => (20, true),
                    Lod::Box | Lod::Far => (8, false),
                };
                self.prism(c, r, r, bottom, eave, sides, 0.0, smooth, &facade, wall);
                match (b.roof, lod) {
                    (Roof::Cone { rise }, Lod::Full) => {
                        self.cone(c, r, eave, eave + rise, sides, 0.0, true, roof)
                    }
                    _ => self.disc(c, r, eave, sides, 0.0, false, roof),
                }
            }
            _ => match lod {
                Lod::Full => {
                    let region = self.clutter;
                    if let Some(region) = region {
                        roof_clutter(b, region, eave, &mut self.mesh.clutter);
                    }
                    let parapet = region.map_or(0.0, |r| parapet_of(b, r));
                    let eaves = region.and_then(|r| eaves_of(b, r));
                    let dormers = |b: &Building, rise: f32| {
                        let Some(region) = region else {
                            return 0;
                        };
                        if ring.len() != 4 {
                            return 0;
                        }
                        let long = geometry::distance(ring[0], ring[1]);
                        let depth = geometry::distance(ring[1], ring[2]) * 0.5;
                        dormers_of(b, region, depth, long, rise)
                    };
                    let top = self.wall_top(b, lod);
                    self.walls(b, ring, bottom, top, PART_WALL);
                    if b.base > 0.0 {
                        self.soffit(b, ring, bottom);
                    }
                    match b.roof {
                        Roof::Flat | Roof::Cone { .. } => {
                            self.flat_roof(b, ring, eave);
                            if parapet > 0.0 {
                                self.parapet(b, ring, eave, parapet);
                            }
                        }
                        Roof::Gabled { rise } => {
                            let n = dormers(b, rise);
                            self.pitched_roof(b, ring, eave, rise, false, eaves, n)
                        }
                        Roof::Hipped { rise } => {
                            let n = dormers(b, rise);
                            self.pitched_roof(b, ring, eave, rise, true, eaves, n)
                        }
                        Roof::Sawtooth { rise, tooth } => {
                            if ring.len() == 4 {
                                self.sawtooth_roof(b, ring, eave, rise, tooth)
                            } else {
                                self.flat_roof(b, ring, eave)
                            }
                        }
                    }
                }
                Lod::Box | Lod::Far => {
                    // Coarser outlines; a pitched roof's mean height as a flat top.
                    let coarse = if ring.len() > 4 {
                        geometry::simplify(ring, 2.0).unwrap_or_else(|| ring.clone())
                    } else {
                        ring.clone()
                    };
                    // A flat roof's parapet on its box too, so the skyline holds from one level to the next.
                    let top = self.wall_top(b, lod);
                    self.walls(b, &coarse, bottom, top, PART_WALL);
                    if b.base > 0.0 {
                        self.soffit(b, &coarse, bottom);
                    }
                    self.flat_roof(b, &coarse, top);
                }
            },
        }
        for e in &b.extras {
            self.extra(b, e, lod);
        }
        self.mesh.buildings += 1;
    }

    /// The chunk's region: its index kept for the facades, and its buildings' roof clutter made in its manner.
    pub fn with_region(mut self, region: Region) -> Self {
        self.mesh.region = region.index();
        self.clutter = Some(region);
        self
    }

    pub fn finish(mut self) -> ChunkMesh {
        self.mesh.clutter_counts = sort_by_shape(&mut self.mesh.clutter);
        // The bounds hold the clutter whole: a water tower over the highest roof, a garden's wall or a downpipe's foot
        // off the buildings (the layer culls the chunk and its shadows by them).
        for c in &self.mesh.clutter {
            let r = 0.5 * c.size[0].hypot(c.size[2]);
            for a in [0, 2] {
                self.mesh.min[a] = self.mesh.min[a].min(c.at[a] - r);
                self.mesh.max[a] = self.mesh.max[a].max(c.at[a] + r);
            }
            self.mesh.min[1] = self.mesh.min[1].min(c.at[1]);
            self.mesh.max[1] = self.mesh.max[1].max(c.at[1] + c.size[1]);
        }
        self.mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::tags::{Landmark, Poi, PoiKind, Religion, Usage, Zone};
    use crate::buildings::{ChunkKey, Setting};

    fn plain(ring: Vec<[f32; 2]>, height: f32, roof: Roof, kind: Kind) -> Building {
        Building {
            ring,
            kind,
            roof,
            ground: [4.0, 5.0, 6.0],
            height,
            base: 0.0,
            storeys: 3,
            storey_m: 3.0,
            wall: 1,
            roof_colour: 2,
            seed: 7,
            detail: false,
            bay: 3.0,
            extras: Vec::new(),
            period: Period::Today,
        }
    }

    fn normal(v: &BuildingVertex) -> [f32; 3] {
        [
            f32::from(v.normal[0]) / 127.0,
            f32::from(v.normal[1]) / 127.0,
            f32::from(v.normal[2]) / 127.0,
        ]
    }

    fn part(v: &BuildingVertex) -> u8 {
        (v.style[3] >> 4) & 7
    }

    /// Every triangle wound counter-clockwise seen from where its normals point (wgpu's front face).
    fn faces_out(mesh: &ChunkMesh) {
        for t in mesh.indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
            let n = cross(sub(b.position, a.position), sub(c.position, a.position));
            if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] < 1e-10 {
                continue;
            }
            let stored = [normal(&a), normal(&b), normal(&c)]
                .iter()
                .fold([0.0; 3], |s, m| [s[0] + m[0], s[1] + m[1], s[2] + m[2]]);
            let d = n[0] * stored[0] + n[1] * stored[1] + n[2] * stored[2];
            assert!(d > 0.0, "a triangle facing in: {t:?} {n:?} {stored:?}");
        }
    }

    #[test]
    fn a_square_extrudes_to_a_closed_box() {
        let ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let mut m = MeshBuilder::default();
        m.building(&plain(ring, 9.0, Roof::Flat, Kind::Block), Lod::Full);
        let mesh = m.finish();
        // Four walls of two triangles and a roof of two.
        assert_eq!(mesh.triangles(), 10);
        assert_eq!(mesh.vertices.len(), 20);
        assert_eq!(mesh.buildings, 1);
        faces_out(&mesh);
        // From 1.5 m under the lowest ground to 9 m over the mean (the highest ground's 2.2 m below that).
        assert_eq!(mesh.min[1], 4.0 - SINK_M);
        assert_eq!(mesh.max[1], 14.0);
        // Walls' normals point out of the square; the roof's up.
        for v in &mesh.vertices {
            let n = normal(v);
            if part(v) == PART_ROOF {
                assert!(n[1] > 0.99);
            } else {
                let p = v.position;
                let out = [p[0] - 5.0, p[2] - 5.0];
                assert!(n[0] * out[0] + n[2] * out[1] > 0.0);
                assert!(n[1].abs() < 1e-3);
            }
        }
        // The top storey's facade: 14 m over a 5 m mean ground at 3 m a storey; a 10 m wall holds three bays.
        let top = mesh
            .vertices
            .iter()
            .filter(|v| part(v) == PART_WALL && v.position[1] == 14.0)
            .map(|v| {
                (
                    f16::from_bits(v.facade[0]).to_f32(),
                    f16::from_bits(v.facade[1]).to_f32(),
                )
            })
            .fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        assert_eq!(top.0, 3.0);
        assert!((top.1 - 3.0).abs() < 0.01);
        // The style byte: kind in the low nibble, part above, detail on top.
        let s = style_of(1, 2, 3, Kind::Deck, PART_DOME, true);
        assert_eq!(s[3] & 15, Kind::Deck as u8);
        assert_eq!((s[3] >> 4) & 7, PART_DOME);
        assert_eq!(s[3] >> 7, 1);
    }

    #[test]
    fn a_wall_against_its_neighbour_stands_only_above_it() {
        // Two pieces of a block back to back along x = 10: a tall one and a low one.
        let square = |x: f32| vec![[x, 0.0], [x + 10.0, 0.0], [x + 10.0, 10.0], [x, 10.0]];
        let pieces = [
            plain(square(0.0), 12.0, Roof::Flat, Kind::Block),
            plain(square(10.0), 6.0, Roof::Flat, Kind::Block),
        ];
        let mut m = MeshBuilder::default();
        let parties = m.parties(&pieces, Lod::Full);
        // Each against the other along x = 10, up to the other's walls' top: the low one's 11 m, the tall one's 17.
        let party = |from: [f32; 2], to: [f32; 2], top: f32| Party { from, to, top };
        assert_eq!(parties[0], [party([10.0, 0.0], [10.0, 10.0], 11.0)]);
        assert_eq!(parties[1], [party([10.0, 10.0], [10.0, 0.0], 17.0)]);
        for (b, party) in pieces.iter().zip(parties) {
            m.set_party(party);
            m.building(b, Lod::Full);
        }
        let mesh = m.finish();
        // The tall one's four walls (the shared one above the low one's top), the low one's other three, two roofs.
        assert_eq!(mesh.triangles(), 2 * 4 + 2 * 3 + 2 * 2);
        faces_out(&mesh);
        for v in &mesh.vertices {
            if (v.position[0] - 10.0).abs() < 1e-4 && normal(v)[0].abs() > 0.9 {
                assert!(v.position[1] >= 11.0 - 1e-4, "{:?}", v.position);
                // The window rows go on from the storeys under it.
                assert!(f16::from_bits(v.facade[1]).to_f32() >= 2.0 - 0.01);
            }
        }
    }

    #[test]
    fn a_blocks_pieces_find_their_neighbours_across_both_rows() {
        // A row of three before a row of two, parted at other places.
        let rect =
            |x0: f32, z0: f32, x1: f32, z1: f32| vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]];
        let rings = [
            rect(0.0, 0.0, 8.0, 10.0),
            rect(8.0, 0.0, 15.0, 10.0),
            rect(15.0, 0.0, 24.0, 10.0),
            rect(0.0, 10.0, 11.0, 20.0),
            rect(11.0, 10.0, 24.0, 20.0),
        ];
        // 6-10 m: their boxes' tops 11-15 m over the sea.
        let pieces: Vec<Building> = rings
            .into_iter()
            .enumerate()
            .map(|(i, ring)| plain(ring, 6.0 + i as f32, Roof::Flat, Kind::Block))
            .collect();
        let m = MeshBuilder::default();
        let parties = m.parties(&pieces, Lod::Box);
        let counts: Vec<usize> = parties.iter().map(Vec::len).collect();
        assert_eq!(counts, [2, 4, 2, 3, 3]);
        let runs: Vec<f32> = parties
            .iter()
            .map(|p| p.iter().map(|s| geometry::distance(s.from, s.to)).sum())
            .collect();
        for (run, want) in runs.iter().zip([18.0, 27.0, 19.0, 21.0, 23.0]) {
            assert!((run - want).abs() < 1e-4, "{runs:?}");
        }
        // The middle front piece (top 12 m): its back behind taller ones and its east side against one gone, its west
        // side above its 11 m neighbour; its front whole.
        let mut m = MeshBuilder::default();
        m.set_party(parties[1].clone());
        m.building(&pieces[1], Lod::Box);
        let mesh = m.finish();
        assert_eq!(mesh.triangles(), 2 + 2 + 2);
        faces_out(&mesh);
    }

    #[test]
    fn an_l_shape_extrudes_with_a_whole_roof() {
        let ring = vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 10.0],
            [10.0, 10.0],
            [10.0, 20.0],
            [0.0, 20.0],
        ];
        let mut m = MeshBuilder::default();
        m.building(
            &plain(ring.clone(), 6.0, Roof::Flat, Kind::Terrace),
            Lod::Full,
        );
        let mesh = m.finish();
        assert_eq!(mesh.triangles(), 6 * 2 + 4);
        faces_out(&mesh);
        // The roof's triangles cover the L's 300 m².
        let roof: f32 = mesh
            .indices
            .chunks(3)
            .filter(|t| part(&mesh.vertices[t[0] as usize]) == PART_ROOF)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize].position);
                let n = cross(sub(b, a), sub(c, a));
                0.5 * n[1]
            })
            .sum();
        assert!((roof - 300.0).abs() < 1e-2, "{roof}");
        // A box chunk draws it the same way (it is already simple).
        let mut m = MeshBuilder::default();
        m.building(&plain(ring, 6.0, Roof::Flat, Kind::Terrace), Lod::Box);
        assert_eq!(m.finish().triangles(), 16);
    }

    #[test]
    fn pitched_roofs_close_over_the_walls() {
        let ring = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
        for roof in [Roof::Gabled { rise: 3.0 }, Roof::Hipped { rise: 3.0 }] {
            let mut m = MeshBuilder::default();
            m.building(&plain(ring.clone(), 5.0, roof, Kind::House), Lod::Full);
            let mesh = m.finish();
            faces_out(&mesh);
            // Walls (8 triangles), two slopes (4), and two gables or hips (2).
            assert_eq!(mesh.triangles(), 14, "{roof:?}");
            // The ridge 3 m over the eaves at 10 m, along the long side: x from 0 to 12 (gabled) or 4 to 8 (hipped).
            let ridge: Vec<[f32; 3]> = mesh
                .vertices
                .iter()
                .map(|v| v.position)
                .filter(|p| (p[1] - 13.0).abs() < 1e-4)
                .collect();
            assert!(ridge.iter().all(|p| (p[2] - 4.0).abs() < 1e-4));
            let xs = ridge.iter().map(|p| p[0]);
            let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |a, x| (a.0.min(x), a.1.max(x)));
            match roof {
                Roof::Gabled { .. } => assert_eq!((lo, hi), (0.0, 12.0)),
                _ => assert_eq!((lo, hi), (4.0, 8.0)),
            }
        }
    }

    #[test]
    fn sawtooth_roofs_and_tanks_close_within_their_footprints() {
        let ring = vec![[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]];
        let mut m = MeshBuilder::default();
        m.building(
            &plain(
                ring.clone(),
                8.0,
                Roof::Sawtooth {
                    rise: 3.0,
                    tooth: 8.0,
                },
                Kind::Industrial,
            ),
            Lod::Full,
        );
        let mesh = m.finish();
        faces_out(&mesh);
        // Walls (8), five teeth of six triangles.
        assert_eq!(mesh.triangles(), 8 + 5 * 6);
        assert!(mesh.vertices.iter().any(|v| part(v) == PART_GLASS));
        for v in &mesh.vertices {
            let p = [v.position[0], v.position[2]];
            assert!(geometry::contains(&ring, p) || geometry::ring_distance(&ring, p) < 1e-3);
            assert!(v.position[1] <= 13.0 + 3.0 + 1e-3);
        }
        // Turned 30 degrees: every face out of the footprint's middle (the teeth's end triangles stand in the long walls'
        // planes, facing out of them).
        let (s, c) = 30f32.to_radians().sin_cos();
        let turned: Vec<[f32; 2]> = [[-25.0, -15.0], [25.0, -15.0], [25.0, 15.0], [-25.0, 15.0]]
            .iter()
            .map(|p: &[f32; 2]| [100.0 + p[0] * c - p[1] * s, 100.0 + p[0] * s + p[1] * c])
            .collect();
        let mut m = MeshBuilder::default();
        m.building(
            &plain(
                turned,
                8.0,
                Roof::Sawtooth {
                    rise: 3.0,
                    tooth: 8.0,
                },
                Kind::Industrial,
            ),
            Lod::Full,
        );
        let mesh = m.finish();
        faces_out(&mesh);
        for v in mesh.vertices.iter().filter(|v| part(v) == PART_GABLE) {
            let n = normal(v);
            let out = [v.position[0] - 100.0, v.position[2] - 100.0];
            assert!(
                n[0] * out[0] + n[2] * out[1] > 0.0,
                "{:?} {n:?}",
                v.position
            );
        }
        let tank_ring = geometry::regular([50.0, 50.0], 15.0, 20, 0.0);
        let mut tank = plain(
            tank_ring.clone(),
            14.0,
            Roof::Cone { rise: 2.0 },
            Kind::Tank,
        );
        tank.ground = [3.0, 3.0, 3.0];
        for (lod, sides) in [(Lod::Full, 20), (Lod::Box, 8)] {
            let mut m = MeshBuilder::default();
            m.building(&tank, lod);
            let mesh = m.finish();
            faces_out(&mesh);
            assert_eq!(
                mesh.triangles(),
                sides * 3 - if lod == Lod::Box { 2 } else { 0 }
            );
            for v in &mesh.vertices {
                let r = (v.position[0] - 50.0).hypot(v.position[2] - 50.0);
                assert!(r <= 15.0 + 1e-3);
            }
        }
    }

    #[test]
    fn eaves_hold_the_roof_out_over_its_walls_and_dormers_stand_on_its_slope() {
        let mut eaves = 0;
        let mut dormers = 0;
        for k in 0..60 {
            let x = k as f32 * 17.0;
            let ring = vec![[x, 0.0], [x + 10.0, 0.0], [x + 10.0, 8.0], [x, 8.0]];
            for (roof, hipped) in [
                (Roof::Gabled { rise: 3.6 }, false),
                (Roof::Hipped { rise: 3.6 }, true),
            ] {
                let mut b = plain(ring.clone(), 5.6, roof, Kind::House);
                b.storeys = 2;
                b.storey_m = 2.8;
                let mut m = MeshBuilder::default().with_region(Region::Celtic);
                m.building(&b, Lod::Full);
                let mesh = m.finish();
                faces_out(&mesh);
                let e = eaves_of(&b, Region::Celtic).unwrap();
                let eave = (b.ground[1] + b.height).max(b.ground[2] + 2.2);
                let tan = 3.6 / 4.0;
                // Nothing past the overhang; the roof's edge at least part of it out.
                let out = mesh
                    .vertices
                    .iter()
                    .map(|v| {
                        let p = v.position;
                        (x - p[0]).max(p[0] - x - 10.0).max(-p[2]).max(p[2] - 8.0)
                    })
                    .fold(f32::MIN, f32::max);
                assert!(out <= e.overhang + 1e-3, "{out} {e:?}");
                assert!(out >= e.overhang - 1e-3, "{out} {e:?}");
                // The soffit under the overhang, below the walls' top by the slope over it; the ridge its thickness up.
                let lowest_trim = mesh
                    .vertices
                    .iter()
                    .filter(|v| part(v) == PART_STEEL)
                    .map(|v| v.position[1])
                    .fold(f32::MAX, f32::min);
                assert!((lowest_trim - (eave - e.overhang * tan - e.gutter)).abs() < 1e-3);
                // (The mesh's bounds hold its chimneys too.)
                let top = mesh
                    .vertices
                    .iter()
                    .map(|v| v.position[1])
                    .fold(f32::MIN, f32::max);
                assert!((top - (eave + 3.6 + e.thickness)).abs() < 1e-3, "{top}");
                eaves += 1;
                // A dormer's face: a window on the attic's storey, under the ridge, inside the walls.
                let attic: Vec<&BuildingVertex> = mesh
                    .vertices
                    .iter()
                    .filter(|v| part(v) == PART_WALL && v.normal[3] == 3)
                    .collect();
                if !attic.is_empty() {
                    dormers += 1;
                    assert!(
                        !hipped
                            || attic
                                .iter()
                                .all(|v| v.position[0] > x + 1.0 && v.position[0] < x + 9.0)
                    );
                    for v in &attic {
                        assert!(v.position[1] > eave && v.position[1] < top);
                        assert!(v.position[2] > 0.0 && v.position[2] < 8.0);
                    }
                }
            }
        }
        assert_eq!(eaves, 120);
        assert!(dormers > 10, "{dormers}");
        // No region (a test's builder, or the far level's): flush, as before.
        let b = plain(
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]],
            5.0,
            Roof::Gabled { rise: 3.0 },
            Kind::House,
        );
        let mut m = MeshBuilder::default();
        m.building(&b, Lod::Full);
        assert_eq!(m.finish().triangles(), 14);
    }

    #[test]
    fn a_parapet_stands_over_a_flat_roof_facing_in() {
        let mut found = 0;
        for k in 0..40 {
            let x = k as f32 * 30.0;
            let b = plain(
                vec![[x, 0.0], [x + 20.0, 0.0], [x + 20.0, 14.0], [x, 14.0]],
                12.0,
                Roof::Flat,
                Kind::Block,
            );
            let parapet = parapet_of(&b, Region::Temperate);
            let mut m = MeshBuilder::default().with_region(Region::Temperate);
            m.building(&b, Lod::Full);
            let mesh = m.finish();
            faces_out_but_inner(&mesh, [x + 10.0, 7.0]);
            let walls = mesh
                .vertices
                .iter()
                .filter(|v| part(v) != PART_ROOF)
                .map(|v| v.position[1])
                .fold(f32::MIN, f32::max);
            assert!((walls - (17.0 + parapet)).abs() < 1e-3, "{walls} {parapet}");
            found += usize::from(parapet > 0.0);
            // Its box stands as tall.
            let mut m = MeshBuilder::default().with_region(Region::Temperate);
            m.building(&b, Lod::Box);
            assert!((m.finish().max[1] - (17.0 + parapet)).abs() < 1e-3);
        }
        assert!(found > 25, "{found}");
    }

    /// As [`faces_out`], but a parapet's inner faces (gables over the roof) face the building's middle `c`.
    fn faces_out_but_inner(mesh: &ChunkMesh, c: [f32; 2]) {
        for t in mesh.indices.chunks(3) {
            let [a, b, d] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
            let n = cross(sub(b.position, a.position), sub(d.position, a.position));
            let stored = normal(&a);
            assert!(n[0] * stored[0] + n[1] * stored[1] + n[2] * stored[2] > 0.0);
            if part(&a) == PART_GABLE {
                let to = [c[0] - a.position[0], c[1] - a.position[2]];
                assert!(stored[0] * to[0] + stored[2] * to[1] > 0.0);
            }
        }
    }

    fn rect(x: f32, z: f32, w: f32, d: f32) -> Footprint {
        Footprint {
            ring: vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]],
            ..Footprint::default()
        }
    }

    /// Taipei 101's shaft, crown and spire as OSM's parts, all of the complex whose middle is (462.5, 462.5).
    fn stacked_tower() -> Vec<Footprint> {
        let part = |x: f32, side: f32, base: f32, top: f32| Footprint {
            ring: vec![[x, x], [x + side, x], [x + side, x + side], [x, x + side]],
            height: Some(top),
            part: Some(super::super::Part {
                base,
                middle: [462.5, 462.5],
            }),
            ..Footprint::default()
        };
        vec![
            part(430.0, 53.0, 0.0, 386.0),
            part(445.0, 23.0, 390.0, 439.0),
            part(455.8, 1.4, 448.0, 508.0),
        ]
    }

    #[test]
    fn a_raised_part_is_closed_underneath() {
        // A skybridge mapped from 20 to 25 m over the mean ground: its floor seen from the street, at both levels.
        let ring = vec![[0.0, 0.0], [30.0, 0.0], [30.0, 6.0], [0.0, 6.0]];
        let bridge = Building {
            base: 20.0,
            ..plain(ring.clone(), 25.0, Roof::Flat, Kind::Office)
        };
        let foot = foot_of(&bridge);
        assert_eq!(foot, 25.0);
        for lod in [Lod::Full, Lod::Box] {
            let mut m = MeshBuilder::default();
            m.building(&bridge, lod);
            let mesh = m.finish();
            faces_out(&mesh);
            assert_eq!(mesh.min[1], foot, "{lod:?}");
            // The ring at its foot, facing down, whole (30 by 6 m).
            let under: f32 = mesh
                .indices
                .chunks(3)
                .map(|t| [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]))
                .filter(|t| {
                    t.iter()
                        .all(|v| normal(v)[1] < -0.99 && v.position[1] == foot)
                })
                .map(|[a, b, c]| {
                    let n = cross(sub(b.position, a.position), sub(c.position, a.position));
                    0.5 * n[1].abs()
                })
                .sum();
            assert!((under - 180.0).abs() < 1e-2, "{lod:?}: {under}");
        }
        // One on the ground has no underside.
        let mut m = MeshBuilder::default();
        m.building(&plain(ring, 25.0, Roof::Flat, Kind::Office), Lod::Full);
        assert!(!m.finish().vertices.iter().any(|v| normal(v)[1] < -0.99));
    }

    #[test]
    fn an_infill_lot_reaching_into_a_clear_area_at_all_is_left_out() {
        let ground = |_: [f32; 2]| 5.0;
        let lot = |x: f32| Footprint {
            ring: vec![[x, 100.0], [x + 20.0, 100.0], [x + 20.0, 120.0], [x, 120.0]],
            height: Some(9.0),
            kind: Some(Kind::Block),
            ..Footprint::default()
        };
        let infill = vec![lot(100.0), lot(200.0), lot(300.0)];
        let mesh = |tags: &MapTags| {
            build_chunk(&ChunkInput {
                key: ChunkKey {
                    index: [0, 0],
                    lod: Lod::Full,
                },
                footprints: &[],
                infill: &infill,
                tags,
                ground: &ground,
                region: Region::Temperate,
                period: Period::Today,
                thinning: None,
            })
        };
        let tags = MapTags {
            cleared: vec![
                // Across the first lot's north wall: none of the lot's corners, nor its middle, in it.
                vec![[109.0, 90.0], [111.0, 90.0], [111.0, 105.0], [109.0, 105.0]],
                // Right through the second: none of either's corners in the other.
                vec![[209.0, 50.0], [211.0, 50.0], [211.0, 150.0], [209.0, 150.0]],
            ],
            ..MapTags::default()
        };
        assert_eq!(mesh(&MapTags::default()).buildings, 3);
        let cleared = mesh(&tags);
        assert_eq!(cleared.buildings, 1);
        assert!(cleared.min[0] >= 290.0, "{:?}", cleared.min);
    }

    #[test]
    fn a_tower_drawn_by_its_parts_stands_from_each_base_to_its_top() {
        let ground = |p: [f32; 2]| 6.0 + 0.01 * (p[0] - 400.0);
        let tags = MapTags::default();
        let footprints = stacked_tower();
        let input = |footprints: &'_ [Footprint], lod: Lod| {
            build_chunk(&ChunkInput {
                key: ChunkKey { index: [0, 0], lod },
                footprints,
                infill: &[],
                tags: &tags,
                ground: &ground,
                region: Region::Temperate,
                period: Period::Today,
                thinning: None,
            })
        };
        // Read at the complex's middle: its ground there.
        let mean = ground([462.5, 462.5]);
        for lod in [Lod::Full, Lod::Box] {
            let mesh = input(&footprints, lod);
            assert_eq!(mesh.buildings, 3, "{lod:?}");
            faces_out(&mesh);
            let top = mesh
                .vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::MIN, f32::max);
            assert!(
                (mean + 508.0..mean + 510.0).contains(&top),
                "{lod:?}: {top}"
            );
            // One building's colours and hash on every part.
            let walls: Vec<[u8; 4]> = mesh
                .vertices
                .iter()
                .filter(|v| (v.style[3] >> 4) & 7 == PART_WALL)
                .map(|v| v.style)
                .collect();
            assert!(
                walls
                    .iter()
                    .all(|s| s[0] == walls[0][0] && s[2] == walls[0][2])
            );
            // The spire reaches the obstruction lights' rule: a lamp at its top.
            let beacons = crate::night::beacons_of(&mesh);
            assert!(
                beacons.iter().any(|b| b[1] > mean + 508.0),
                "{lod:?}: {beacons:?}"
            );
            // The crown alone: its walls start at its base, nothing under it, nothing sunk.
            let crown = input(&footprints[1..2], lod);
            assert_eq!(crown.buildings, 1);
            assert!(
                (crown.min[1] - (mean + 390.0)).abs() < 1e-3,
                "{lod:?}: {}",
                crown.min[1]
            );
            // The shaft from below the ground, as any building.
            let shaft = input(&footprints[..1], lod);
            assert!(shaft.min[1] < ground([430.0, 430.0]) - SINK_M + 0.01);
        }
    }

    #[test]
    fn a_chunk_plans_and_builds_its_footprints() {
        let ground = |p: [f32; 2]| 3.0 + p[0] * 0.001;
        let footprints: Vec<Footprint> = (0..10)
            .map(|k| rect(100.0 + 30.0 * k as f32, 100.0, 9.0, 11.0))
            .collect();
        let tags = MapTags::default();
        let input = ChunkInput {
            key: ChunkKey {
                index: [3, -2],
                lod: Lod::Full,
            },
            footprints: &footprints,
            infill: &[],
            tags: &tags,
            ground: &ground,
            region: Region::Nordic,
            period: Period::Today,
            thinning: None,
        };
        let a = build_chunk(&input);
        assert_eq!(a.buildings, 10);
        faces_out(&a);
        // Deterministic.
        let b = build_chunk(&input);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
        let boxes = build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [3, -2],
                lod: Lod::Box,
            },
            ..input
        });
        assert!(boxes.triangles() < a.triangles());
        faces_out(&boxes);
    }

    #[test]
    fn the_map_raises_churches_lighthouses_cranes_and_piers() {
        // The sea west of x = 200; an industrial waterfront from x = 200 to 600.
        let ground = |p: [f32; 2]| if p[0] < 200.0 { -6.0 } else { 3.0 };
        let footprints = vec![rect(400.0, 600.0, 30.0, 14.0), rect(700.0, 700.0, 8.0, 8.0)];
        let tags = MapTags {
            zones: vec![Zone::new(
                Usage::Industrial,
                vec![
                    [200.0, 100.0],
                    [600.0, 100.0],
                    [600.0, 500.0],
                    [200.0, 500.0],
                ],
                vec![],
            )],
            pois: vec![Poi {
                at: [415.0, 607.0],
                kind: PoiKind::Worship(Religion::Christian),
            }],
            piers: vec![Pier::line(&[[205.0, 550.0], [100.0, 550.0]]).unwrap()],
            landmarks: vec![
                // A lighthouse with no footprint, and one on the 8 m footprint (which it takes).
                Landmark {
                    at: [150.0, 900.0],
                    kind: LandmarkKind::Lighthouse {
                        height: 20.0,
                        bands: 2,
                        red: true,
                        keeper: false,
                    },
                },
                Landmark {
                    at: [704.0, 704.0],
                    kind: LandmarkKind::Lighthouse {
                        height: 30.0,
                        bands: 0,
                        red: false,
                        keeper: false,
                    },
                },
            ],
            port: true,
            ..MapTags::default()
        };
        let mesh = build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [0, 0],
                lod: Lod::Full,
            },
            footprints: &footprints,
            infill: &[],
            tags: &tags,
            ground: &ground,
            region: Region::Temperate,
            period: Period::Today,
            thinning: None,
        });
        faces_out(&mesh);
        let kind_of = |v: &BuildingVertex| v.style[3] & 15;
        let has = |k: Kind| mesh.vertices.iter().any(|v| kind_of(v) == k as u8);
        assert!(has(Kind::Church) && has(Kind::Lighthouse) && has(Kind::Crane) && has(Kind::Deck));
        // The church's tower stands well over its nave.
        let church_top = mesh
            .vertices
            .iter()
            .filter(|v| kind_of(v) == Kind::Church as u8)
            .map(|v| v.position[1])
            .fold(0.0, f32::max);
        assert!(church_top > 25.0, "{church_top}");
        // The lighthouse in the sea stands on a plinth; its lantern's glass is there, lit.
        assert!(mesh.vertices.iter().any(|v| {
            kind_of(v) == Kind::Lighthouse as u8 && part(v) == PART_GLASS && v.style[3] >> 7 == 1
        }));
        // The 8 m footprint taken by its lighthouse: nothing of a house there.
        assert!(!mesh.vertices.iter().any(|v| {
            kind_of(v) == Kind::House as u8
                && (v.position[0] - 704.0).abs() < 5.0
                && (v.position[2] - 704.0).abs() < 5.0
        }));
        // The cranes stand on the quay, 12 m back from its edge, on land.
        for v in mesh
            .vertices
            .iter()
            .filter(|v| kind_of(v) == Kind::Crane as u8)
        {
            assert!(v.position[1] >= 3.0 - 0.6, "{:?}", v.position);
        }
        // A box chunk drops the cranes.
        let boxes = build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [0, 0],
                lod: Lod::Box,
            },
            footprints: &footprints,
            infill: &[],
            tags: &tags,
            ground: &ground,
            region: Region::Temperate,
            period: Period::Today,
            thinning: None,
        });
        assert!(
            !boxes
                .vertices
                .iter()
                .any(|v| kind_of(v) == Kind::Crane as u8)
        );
        assert!(boxes.triangles() < mesh.triangles());
        // A setting off the map's tags: the pier holds its deck height.
        let s: Setting = tags.setting(&rect(150.0, 548.0, 3.0, 3.0).ring, &ground);
        assert_eq!(s.pier, Some(1.1));
        // The whole chunk an area the host draws itself: no church, lighthouse, landmark, crane or pier stands.
        let cleared = MapTags {
            cleared: vec![vec![
                [-10.0, -10.0],
                [1010.0, -10.0],
                [1010.0, 1010.0],
                [-10.0, 1010.0],
            ]],
            ..tags.clone()
        };
        let empty = build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [0, 0],
                lod: Lod::Full,
            },
            footprints: &footprints,
            infill: &[],
            tags: &cleared,
            ground: &ground,
            region: Region::Temperate,
            period: Period::Today,
            thinning: None,
        });
        assert_eq!((empty.buildings, empty.cranes), (0, 0));
        assert!(empty.vertices.is_empty() && empty.clutter.is_empty());
    }
}
