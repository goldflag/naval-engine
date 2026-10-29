//! The trees as meshes (native/docs/terrain/trees.md, "Near trees", "Grown trees"): every species' grown tree
//! ([`crate::species::models`]: its limbs and its sprays, [`crate::tree_growth`]) turned into geometry once a process,
//! generically, so a new species needs no work of its own. Each limb becomes a tapering tube (a trunk flared into its
//! roots); each spray a card carrying an alpha-tested branchlet of leaves, needles or bare twigs from the leaf sprites
//! ([`leaf_sprites`]), its stem on its branch; a run of clumps strung along a line (a palm's frond, a blade of grass:
//! [`chains`]) one ribbon down its length. The sprays of the clumps buried deepest in a crown are left out, and the most
//! open come first in the mesh: a GPU sets up and runs fragments for every card over every pixel it covers, leaves or
//! not, and that, not the vertices, is what near trees cost. Two levels of detail: the full one near the camera (and
//! the impostors are baked from it, `impostors`), and two in five sprays in larger cards beyond it, out to where the
//! impostor takes over (`shaders/near_trees.wgsl`).
//!
//! The vertices stay in the model's own space (foot at 0, top at 1, the axis at x = z = 0), so a placed tree's height,
//! width and turn place them as they place its impostor; the leaves' normals lean from each clump's centre toward its
//! lobe's, so a crown shades as masses of leaves.

use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::crown::CrownLight;
use crate::species::{KIND_BARK, Model, Primitive, Species, models};
use crate::tree_growth::{Limb, Spray, corners};

/// Levels of detail each species' mesh has.
pub const MESH_LODS: usize = 2;

/// The leaf sprites' grid on each of their layers of the impostor atlas: `SPRITE_GRID`² slots a layer.
pub const SPRITE_GRID: u32 = 2;
/// The leaf sprites' layers (`impostors::LEAF_LAYERS`).
pub const SPRITE_LAYERS: u32 = 3;

/// The sprite a species' leaves are drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Foliage {
    /// A twig of ovate leaves (beech, lime, ash, birch, the temperate broadleaves).
    Broadleaf = 0,
    /// Fine twigs of small leaves (live oak, acacia, the shrubs, mangrove, chaparral).
    SmallLeaf = 1,
    /// A flat spray of short needles (spruce, fir, redwood, sugi).
    Needles = 2,
    /// A pinnate frond (a palm's): leaflets down both sides of a rachis.
    Frond = 3,
    /// Bare twigs (deciduous trees in winter).
    Twigs = 4,
    /// Blades of grass.
    Grass = 5,
    /// Lobed oak leaves in rosettes at the twigs' ends (oak, sycamore).
    Lobed = 6,
    /// Tufts of long pine needles at the branchlets' ends.
    PineTuft = 7,
    /// Feathery sprays of scale leaves (cypresses).
    Scale = 8,
    /// A palmate fan (the fan palm's).
    Fan = 9,
    /// Long narrow leaves (eucalyptus, olive, bamboo).
    Lanceolate = 10,
    /// The rainforest's large glossy leaves (and the laurel forest's).
    Tropical = 11,
}

impl Foliage {
    pub const ALL: [Foliage; 12] = [
        Foliage::Broadleaf,
        Foliage::SmallLeaf,
        Foliage::Needles,
        Foliage::Frond,
        Foliage::Twigs,
        Foliage::Grass,
        Foliage::Lobed,
        Foliage::PineTuft,
        Foliage::Scale,
        Foliage::Fan,
        Foliage::Lanceolate,
        Foliage::Tropical,
    ];
}

const _: () = assert!(Foliage::ALL.len() as u32 <= SPRITE_GRID * SPRITE_GRID * SPRITE_LAYERS);

/// The sprite a species' leaves take.
pub fn foliage(species: Species) -> Foliage {
    match species {
        Species::Spruce | Species::Redwood | Species::Sugi => Foliage::Needles,
        Species::ScotsPine | Species::StonePine | Species::MontereyPine | Species::BlackPine => {
            Foliage::PineTuft
        }
        Species::Cypress | Species::MontereyCypress => Foliage::Scale,
        Species::CoconutPalm | Species::DatePalm | Species::OasisPalm => Foliage::Frond,
        Species::FanPalm => Foliage::Fan,
        Species::BareOak | Species::BareBirch => Foliage::Twigs,
        Species::GrassTuft => Foliage::Grass,
        Species::Oak | Species::Sycamore => Foliage::Lobed,
        Species::Olive | Species::Eucalyptus | Species::Bamboo => Foliage::Lanceolate,
        Species::Rainforest | Species::Emergent | Species::Laurel => Foliage::Tropical,
        Species::LiveOak
        | Species::Acacia
        | Species::Shrub
        | Species::Garrigue
        | Species::Mangrove
        | Species::Chaparral => Foliage::SmallLeaf,
        Species::Ash | Species::Birch => Foliage::Broadleaf,
    }
}

/// How a species' bark looks close to (`near_trees.wgsl`'s `bark_of`): its pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Bark {
    /// Deep ridges and furrows running up the trunk (oak, ash, olive, acacia).
    Furrowed = 0,
    /// Smooth, a little mottled (the rainforest's, eucalyptus, plane, laurel, the shrubs).
    Smooth = 1,
    /// White with dark horizontal marks (birch).
    Birch = 2,
    /// Scaly plates (the pines).
    Plates = 3,
    /// Long fibrous strips (redwood, cypress, sugi).
    Fibrous = 4,
    /// Rings of old leaf bases (palms) and nodes (bamboo).
    Rings = 5,
}

pub fn bark(species: Species) -> Bark {
    match species {
        Species::Birch | Species::BareBirch => Bark::Birch,
        Species::Rainforest
        | Species::Emergent
        | Species::Mangrove
        | Species::Eucalyptus
        | Species::Sycamore
        | Species::Laurel
        | Species::Shrub
        | Species::Garrigue
        | Species::Chaparral => Bark::Smooth,
        Species::ScotsPine
        | Species::StonePine
        | Species::MontereyPine
        | Species::BlackPine
        | Species::Spruce => Bark::Plates,
        Species::Redwood | Species::Sugi | Species::Cypress | Species::MontereyCypress => {
            Bark::Fibrous
        }
        Species::CoconutPalm
        | Species::FanPalm
        | Species::DatePalm
        | Species::OasisPalm
        | Species::Bamboo => Bark::Rings,
        _ => Bark::Furrowed,
    }
}

/// A bark pattern's ridges' width round a limb (m).
fn ridge_m(pattern: Bark) -> f32 {
    match pattern {
        Bark::Furrowed => 0.13,
        Bark::Smooth => 0.35,
        Bark::Birch => 0.5,
        Bark::Plates => 0.3,
        Bark::Fibrous => 0.05,
        Bark::Rings => 0.4,
    }
}

/// A limb's coordinates (`tree_surface.wgsl`'s `BARK_RIDGES`, `BARK_ALONG_M`): ridges round it over the first, metres
/// along it over the second.
pub(crate) const BARK_RIDGES: f32 = 128.0;
pub(crate) const BARK_ALONG_M: f32 = 64.0;

/// How far a species' foot spreads into buttresses and roots (a share of its radius).
fn buttress(species: Species) -> f32 {
    match species {
        Species::Rainforest | Species::Emergent => 0.9,
        Species::Oak | Species::LiveOak | Species::Redwood | Species::Sycamore => 0.45,
        Species::Mangrove
        | Species::Bamboo
        | Species::CoconutPalm
        | Species::FanPalm
        | Species::OasisPalm => 0.1,
        _ => 0.3,
    }
}

/// One vertex of a tree's mesh, as `shaders/near_trees.wgsl` reads it (36 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct MeshVertex {
    /// Model space: the foot at 0, the top at y = 1, the axis at x = z = 0.
    pub position: [f32; 3],
    /// The shading normal (snorm): leaves their clump's, bark its surface's; w: the level of detail (0 the full one,
    /// 127 the reduced one).
    pub normal: [i8; 4],
    /// Leaves: a card's direction across its sprite (snorm, u growing), w its sprite's layer times 32. Bark: along
    /// the limb.
    pub tangent: [i8; 4],
    /// The leaf sprite's coordinates on its layer (unorm); bark: its ridges round the limb over 128 and metres along
    /// it over 64.
    pub uv: [u16; 2],
    /// The albedo (sRGB) and the share of the sky the point sees through its crown (0 none, 255 open:
    /// [`crate::crown`]).
    pub color: [u8; 4],
    /// x: what it is (0 bark, 128 a card of leaves, 255 twigs); y: leaves the sprite's alpha cut, bark its pattern
    /// ([`Bark`]) times 32; z: its phase in the wind; w: how far it flutters.
    pub wind: [u8; 4],
    /// The slope of the sky the point sees through its crown: toward a direction `d` above the horizon it sees about
    /// `sky + slope · (d - crown::SKY_MEAN)` of it (snorm over [`crate::crown::SLOPE_RANGE`]); w unused.
    pub sky_slope: [i8; 4],
}

pub const MESH_VERTEX_BYTES: u64 = std::mem::size_of::<MeshVertex>() as u64;

/// One species' mesh at one level of detail, within [`TreeMeshes`]' buffers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshRange {
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
}

/// Every species' meshes, in [`Species::ALL`]'s order, both levels of detail each, in one vertex and one index list.
#[derive(Clone, Debug)]
pub struct TreeMeshes {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    /// Per species, per level of detail.
    pub ranges: Vec<[MeshRange; MESH_LODS]>,
}

/// Every species' meshes, built once.
pub fn tree_meshes() -> &'static TreeMeshes {
    static MESHES: OnceLock<TreeMeshes> = OnceLock::new();
    MESHES.get_or_init(|| {
        let mut all = TreeMeshes {
            vertices: Vec::new(),
            indices: Vec::new(),
            ranges: Vec::new(),
        };
        for levels in species_meshes(models()) {
            let mut ranges = [MeshRange::default(); MESH_LODS];
            for (mesh, range) in levels.into_iter().zip(ranges.iter_mut()) {
                *range = MeshRange {
                    first_index: all.indices.len() as u32,
                    index_count: mesh.indices.len() as u32,
                    base_vertex: all.vertices.len() as i32,
                };
                all.vertices.extend_from_slice(&mesh.vertices);
                all.indices.extend_from_slice(&mesh.indices);
            }
            all.ranges.push(ranges);
        }
        all
    })
}

/// One species' meshes at every level of detail, its crown's light ([`CrownLight`]) taken once for both.
fn species_levels(model: &Model) -> [Mesh; MESH_LODS] {
    let light = CrownLight::new(model);
    std::array::from_fn(|lod| build(model, lod, &light))
}

/// Every species' meshes, in order: on wasm32 one after another, natively a species at a time on each of a few
/// threads (the crowns' light is most of the work).
#[cfg(target_arch = "wasm32")]
fn species_meshes(models: &[Model]) -> Vec<[Mesh; MESH_LODS]> {
    models.iter().map(species_levels).collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn species_meshes(models: &[Model]) -> Vec<[Mesh; MESH_LODS]> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8)
        .min(models.len());
    if threads <= 1 {
        return models.iter().map(species_levels).collect();
    }
    let next = AtomicUsize::new(0);
    let mut built: Vec<(usize, [Mesh; MESH_LODS])> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(model) = models.get(k) else {
                            break mine;
                        };
                        mine.push((k, species_levels(model)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("a tree mesh thread panicked"))
            .collect()
    });
    built.sort_by_key(|(k, _)| *k);
    built.into_iter().map(|(_, levels)| levels).collect()
}

/// The web's LCG, as the species' models use.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

struct Mesh {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
}

fn snorm(v: Vec3, w: i8) -> [i8; 4] {
    let n = v.normalize_or(Vec3::Y);
    let q = |x: f32| (x.clamp(-1.0, 1.0) * 127.0).round() as i8;
    [q(n.x), q(n.y), q(n.z), w]
}

/// A slope of the sky through the crown as a vertex carries it.
fn slope_bytes(slope: Vec3) -> [i8; 4] {
    let q = |x: f32| ((x / crate::crown::SLOPE_RANGE).clamp(-1.0, 1.0) * 127.0).round() as i8;
    [q(slope.x), q(slope.y), q(slope.z), 0]
}

fn unorm8(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn unorm16(x: f32) -> u16 {
    (x.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn srgb_encode(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn color_bytes(c: [f32; 3], sky: f32) -> [u8; 4] {
    [
        unorm8(srgb_encode(c[0])),
        unorm8(srgb_encode(c[1])),
        unorm8(srgb_encode(c[2])),
        unorm8(sky),
    ]
}

/// A sprite's slot: its layer among the leaf layers, and its rectangle on that layer (0-1).
pub fn sprite_slot(foliage: Foliage) -> (u32, [f32; 4]) {
    let index = foliage as u32;
    let per_layer = SPRITE_GRID * SPRITE_GRID;
    let slot = index % per_layer;
    let size = 1.0 / SPRITE_GRID as f32;
    let x = (slot % SPRITE_GRID) as f32 * size;
    let y = (slot / SPRITE_GRID) as f32 * size;
    (index / per_layer, [x, y, x + size, y + size])
}

const VERTEX_BARK: u8 = 0;
const VERTEX_LEAVES: u8 = 128;
const VERTEX_TWIGS: u8 = 255;

/// The sprite's alpha cut a spray's leaves take.
const ALPHA_CUT: u8 = 128;
/// A bare crown's twig cards' cut, over their sprite's levels averaged plainly (`sprite_levels`): up close its twigs,
/// a little fuller; further off, where a texel holds many twigs, the mass of them as a soft clump (at the leaves' cut
/// and coverage, a far bare crown kept only its sparse twigs and drew as its limbs).
const TWIG_CUT: u8 = 26;

impl Mesh {
    fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn quad(&mut self, corners: [MeshVertex; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&corners);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn append(&mut self, other: Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }
}

/// Bark thinner than this (m, at the species' own height) is left out, by level of detail, and how many sides a tube
/// takes.
const BARK_LEAST_M: [f32; MESH_LODS] = [0.018, 0.14];

fn bark_sides(radius_m: f32, lod: usize) -> u32 {
    // At full detail no limb is a square prism: it is seen from a few metres, inside the crown.
    match (lod, radius_m) {
        (0, r) if r >= 0.4 => 14,
        (0, r) if r >= 0.15 => 9,
        (0, r) if r >= 0.05 => 6,
        (0, _) => 4,
        (_, r) if r >= 0.4 => 6,
        (_, r) if r >= 0.12 => 4,
        _ => 3,
    }
}

/// A limb as a tube of rings along its axis (each ring's frame carried from the last, so it never twists), its ends
/// pushed out a little so a branch meets its parent with no gap. A trunk on the ground flares into its roots over its
/// first metres (buttresses where the species has them) and reaches a little under the ground.
fn tube(
    mesh: &mut Mesh,
    limb: &Limb,
    species: Species,
    height: f32,
    lod: usize,
    light: &CrownLight,
) {
    let radius_m = limb.radii.iter().copied().fold(0.0f32, f32::max) * height;
    let on_ground = limb.flare && limb.order == 0;
    if radius_m < BARK_LEAST_M[lod] && !(limb.points[0].y < 0.01 && limb.order == 0) {
        return;
    }
    let sides = bark_sides(radius_m, lod);
    let lod_w = if lod == 0 { 0 } else { 127 };
    let pattern = bark(species);
    let style = pattern as u8 * 32;
    // Ridges round the limb, a whole number of them (the pattern meets itself at the seam), narrowing as it tapers.
    let mean_m = limb.radii.iter().sum::<f32>() / limb.radii.len() as f32 * height;
    let ridges = (std::f32::consts::TAU * mean_m / ridge_m(pattern))
        .round()
        .clamp(3.0, BARK_RIDGES - 1.0);
    let mut points = limb.points.clone();
    let mut radii = limb.radii.clone();
    // A branch grows out of its parent: its first point a little inside it.
    if !on_ground && points.len() >= 2 {
        let dir = (points[1] - points[0]).normalize_or(Vec3::Y);
        points[0] -= dir * radii[0] * 0.8;
    }
    if on_ground && lod == 0 {
        // The flare: rings at the foot's first metres, the foot itself a little under the ground.
        let up = (points[1] - points[0]).normalize_or(Vec3::Y);
        let r0 = radii[0];
        let first_span = points[0].distance(points[1]);
        let mut flare_points = vec![points[0] - up * (0.25 / height)];
        let mut flare_radii = vec![r0];
        for metres in [0.0f32, 0.25, 0.6, 1.1, 1.8] {
            let d = metres / height;
            if d >= first_span * 0.9 {
                break;
            }
            flare_points.push(points[0] + up * d);
            flare_radii.push(r0);
        }
        flare_points.extend_from_slice(&points[1..]);
        flare_radii.extend_from_slice(&radii[1..]);
        points = flare_points;
        radii = flare_radii;
    }
    let n = points.len();
    if n < 2 {
        return;
    }
    let ends_out = radii[n - 1] * 0.6;
    let last_dir = (points[n - 1] - points[n - 2]).normalize_or(Vec3::Y);
    points[n - 1] += last_dir * ends_out;
    let color = color_bytes(limb.color, 1.0);
    let foot = points[0];
    let spread = buttress(species);
    let base = mesh.vertices.len() as u32;
    let first_dir = (points[1] - points[0]).normalize_or(Vec3::Y);
    let mut side = if first_dir.y.abs() < 0.9 {
        first_dir.cross(Vec3::Y).normalize()
    } else {
        first_dir.cross(Vec3::X).normalize()
    };
    let mut along_m = 0.0;
    for k in 0..n {
        let dir = if k == 0 {
            points[1] - points[0]
        } else if k + 1 == n {
            points[k] - points[k - 1]
        } else {
            points[k + 1] - points[k - 1]
        }
        .normalize_or(Vec3::Y);
        if k > 0 {
            along_m += points[k].distance(points[k - 1]) * height;
        }
        // Carry the frame: the last side made perpendicular to this ring's axis.
        side = (side - dir * side.dot(dir)).normalize_or(dir.any_orthonormal_vector());
        let other = dir.cross(side);
        // Over the foot's first two metres, the flare and the buttresses.
        let up_m = (points[k].y - foot.y).max(0.0) * height;
        let flare = if on_ground && lod == 0 {
            (-(up_m - 0.25).max(0.0) / 0.45).exp()
        } else {
            0.0
        };
        for j in 0..=sides {
            let angle = j as f32 / sides as f32 * std::f32::consts::TAU;
            let out = side * angle.cos() + other * angle.sin();
            let lobe = (angle * 5.0 + 0.7).cos().max(0.0).powf(1.5);
            let r = radii[k] * (1.0 + flare * (0.3 + spread * lobe));
            // The flared foot's normal leans up out of the ground.
            let normal = (out + dir * flare * 0.5).normalize_or(out);
            let position = points[k] + out * r;
            let (sky, slope) = light.at(position);
            mesh.vertices.push(MeshVertex {
                position: position.to_array(),
                normal: snorm(normal, lod_w),
                tangent: snorm(dir, 0),
                uv: [
                    unorm16(j as f32 / sides as f32 * ridges / BARK_RIDGES),
                    unorm16(along_m / BARK_ALONG_M),
                ],
                color: [color[0], color[1], color[2], unorm8(sky)],
                wind: [VERTEX_BARK, style, 0, 0],
                sky_slope: slope_bytes(slope),
            });
        }
    }
    for k in 0..n as u32 - 1 {
        for j in 0..sides {
            let i = base + k * (sides + 1) + j;
            let next = i + sides + 1;
            mesh.indices
                .extend_from_slice(&[i, i + 1, next, i + 1, next + 1, next]);
        }
    }
}

/// The normal a point of a clump's leaves takes: leaning from the clump's centre toward its lobe's, so a crown shades
/// as masses of leaves (how deep the point lies in the crown is its light's, [`CrownLight`]).
fn clump_normal(p: &Primitive, at: Vec3) -> Vec3 {
    let centre = Vec3::new(p.a[0], p.a[1], p.a[2]);
    let lobe_centre = Vec3::new(p.b[0], p.b[1], p.b[2]);
    let outward = (at - centre).normalize_or(Vec3::Y);
    let lobe = (at - lobe_centre + Vec3::new(0.0, 1e-5, 0.0)).normalize_or(Vec3::Y);
    outward.lerp(lobe, p.d[1]).normalize_or(Vec3::Y)
}

/// A spray's card: its stem at the bottom of its sprite, its tip at the top.
#[allow(clippy::too_many_arguments)]
fn spray_card(
    mesh: &mut Mesh,
    model: &Model,
    spray: &Spray,
    sprite: (u32, [f32; 4]),
    lod: usize,
    light: &CrownLight,
) {
    let p = &model.primitives[spray.clump as usize];
    let twigs = foliage(model.species) == Foliage::Twigs;
    let kind = if twigs { VERTEX_TWIGS } else { VERTEX_LEAVES };
    let lod_w = if lod == 0 { 0 } else { 127 };
    let (layer, rect) = sprite;
    let code = layer * 32 + if lod == 0 { 0 } else { TILED_MARK };
    let color = [
        p.c[0] * spray.tone,
        p.c[1] * spray.tone,
        p.c[2] * spray.tone,
    ];
    let [c0, c1, c2, c3] = corners(spray);
    // A reduced card reaches past the sprays it stands for: a corner standing out of its clump takes the light at the
    // clump's surface, where its outermost leaves are (past it the crown's light sees the open sky, which none of the
    // clump's leaves do, and seen from above a reduced crown's tops drew pale).
    let centre = Vec3::new(p.a[0], p.a[1], p.a[2]);
    let corner = |at: Vec3, s: f32, t: f32| {
        let normal = clump_normal(p, at);
        let out = at - centre;
        let lit_at = if lod == 1 && out.length() > p.a[3] {
            centre + out * (p.a[3] / out.length())
        } else {
            at
        };
        let (sky, slope) = light.at(lit_at);
        MeshVertex {
            position: at.to_array(),
            normal: snorm(normal, lod_w),
            tangent: snorm(spray.across, code as i8),
            uv: [
                unorm16(rect[0] + (rect[2] - rect[0]) * s),
                unorm16(rect[3] - (rect[3] - rect[1]) * t),
            ],
            color: color_bytes(color, sky),
            // The tip flutters, the stem holds.
            wind: [
                kind,
                if twigs { TWIG_CUT } else { ALPHA_CUT },
                unorm8(spray.phase),
                unorm8(0.25 + 0.75 * t),
            ],
            sky_slope: slope_bytes(slope),
        }
    };
    mesh.quad([
        corner(c0, 0.0, 0.0),
        corner(c1, 1.0, 0.0),
        corner(c2, 1.0, 1.0),
        corner(c3, 0.0, 1.0),
    ]);
}

/// Runs of clumps strung along a line, as a palm's fronds and a tuft's blades are built: consecutive leaf clumps of
/// one lobe, evenly spaced, each step turning little from the last, six or more in a run. A crown's clumps, scattered
/// in a ball, almost never line up so. Returns the runs' first indices and lengths (in the model's shapes).
pub fn chains(model: &Model) -> Vec<(usize, usize)> {
    let p = &model.primitives;
    let leafy = |i: usize| p[i].b[3] != KIND_BARK;
    let centre = |i: usize| Vec3::new(p[i].a[0], p[i].a[1], p[i].a[2]);
    let lobe = |i: usize| Vec3::new(p[i].b[0], p[i].b[1], p[i].b[2]);
    // Whether shape `i + 1` continues the run ending at `i` (whose last step was `last`).
    let follows = |i: usize, last: Option<Vec3>| -> bool {
        let j = i + 1;
        if j >= p.len() || !leafy(i) || !leafy(j) || lobe(i).distance(lobe(j)) > 1e-5 {
            return false;
        }
        let step = centre(j) - centre(i);
        // The first step within a few radii, each after it within twice the last.
        let reach = last.map_or(4.0 * p[i].a[3].max(p[j].a[3]), |l| 1.8 * l.length());
        if step.length() > reach || step.length() < 1e-5 {
            return false;
        }
        match last {
            None => true,
            Some(last) => {
                let ratio = step.length() / last.length().max(1e-6);
                (0.55..1.8).contains(&ratio) && step.normalize().dot(last.normalize()) > 0.8
            }
        }
    };
    let mut runs = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let start = i;
        let mut last = None;
        while follows(i, last) {
            last = Some(centre(i + 1) - centre(i));
            i += 1;
        }
        let length = i + 1 - start;
        if length >= 6 {
            runs.push((start, length));
        }
        i += 1;
    }
    runs
}

/// A run of clumps (a frond, a blade) as one ribbon down its length, folded up along its middle (a frond's leaflets
/// rise from the rachis; a coconut palm's hang from it in a V turned down, so its fronds read narrow and feathered
/// from the side, each half lit its own way, not as straps), the sprite's base at the run's first clump.
#[allow(clippy::too_many_arguments)]
fn ribbon(
    mesh: &mut Mesh,
    model: &Model,
    first: usize,
    length: usize,
    sprite: (u32, [f32; 4]),
    phase: f32,
    lod: usize,
    light: &CrownLight,
) {
    let p = &model.primitives;
    let (layer, rect) = sprite;
    let step = if lod == 0 { 1 } else { 2 };
    let mut points: Vec<usize> = (first..first + length).step_by(step).collect();
    if *points.last().unwrap() != first + length - 1 {
        points.push(first + length - 1);
    }
    let centre = |i: usize| Vec3::new(p[i].a[0], p[i].a[1], p[i].a[2]);
    // Reach back toward where the run grows from, by one step.
    let start = centre(first) - (centre(first + 1) - centre(first));
    let mut spine: Vec<(Vec3, f32, usize)> = vec![(start, p[first].a[3], first)];
    spine.extend(points.iter().map(|&i| (centre(i), p[i].a[3], i)));
    let lod_w = if lod == 0 { 0 } else { 127 };
    let lobe_centre = Vec3::new(p[first].b[0], p[first].b[1], p[first].b[2]);
    let count = spine.len();
    // One side for the whole run, across its overall direction, so the ribbon never twists where it stands upright.
    let overall = (spine[count - 1].0 - spine[0].0).normalize_or(Vec3::Y);
    let mut reference = overall.cross(Vec3::Y);
    if reference.length() < 1e-3 {
        reference = overall.cross(Vec3::X);
    }
    let reference = reference.normalize();
    let base = mesh.vertices.len() as u32;
    for (k, &(at, radius, index)) in spine.iter().enumerate() {
        let tangent = if k + 1 < count {
            spine[k + 1].0 - at
        } else {
            at - spine[k - 1].0
        }
        .normalize_or(Vec3::Y);
        let side = (reference - tangent * reference.dot(tangent)).normalize_or(reference);
        let mut up = side.cross(tangent).normalize_or(Vec3::Y);
        if up.y < 0.0 {
            up = -up;
        }
        let half = radius * 1.6;
        let t = k as f32 / (count - 1).max(1) as f32;
        let shape = &p[index];
        let outward = (at - lobe_centre).normalize_or(Vec3::Y);
        let normal = (up * 0.65 + outward * 0.35).normalize_or(Vec3::Y);
        let fold = if model.species == Species::CoconutPalm {
            -0.55
        } else {
            0.3
        };
        for (across, lift) in [(-1.0, fold), (0.0, 0.0), (1.0, fold)] {
            let position = at + side * (across * half) + up * (lift * half);
            let (sky, slope) = light.at(position);
            mesh.vertices.push(MeshVertex {
                position: position.to_array(),
                normal: snorm(normal, lod_w),
                tangent: snorm(side, (layer * 32) as i8),
                uv: [
                    unorm16(rect[0] + (rect[2] - rect[0]) * (across * 0.5 + 0.5)),
                    unorm16(rect[3] - (rect[3] - rect[1]) * t),
                ],
                color: color_bytes([shape.c[0], shape.c[1], shape.c[2]], sky),
                wind: [VERTEX_LEAVES, 110, unorm8(phase), unorm8(0.3 + 0.7 * t)],
                sky_slope: slope_bytes(slope),
            });
        }
    }
    for k in 0..count as u32 - 1 {
        let i = base + k * 3;
        for (a, b) in [(0, 1), (1, 2)] {
            mesh.indices
                .extend_from_slice(&[i + a, i + 3 + a, i + 3 + b, i + a, i + 3 + b, i + b]);
        }
    }
}

/// The least ambient occlusion (0 hidden, 1 open) a clump keeps its sprays with: those buried deeper are hidden from
/// every side by the clumps about them.
pub(crate) const BURIED_LEAST: f32 = 0.24;
/// The share of a crown's sprays the reduced detail keeps, at random, and how many sprays a side their cards are (most
/// of the area of leaves, in far fewer cards): most near trees are at that level, a dense wood's hundreds of them at
/// once. A reduced card carries its sprite as many times over each way (`near_trees.wgsl`'s `REDUCED_TILES`), so its
/// leaves keep their size; its sprite's layer in its tangent's w is marked by [`TILED_MARK`] over the layer's 32s.
pub const REDUCED_SHARE: f32 = 0.1;
pub const REDUCED_TILES: u32 = 3;
const REDUCED_GROWTH: f32 = REDUCED_TILES as f32;
pub const TILED_MARK: u32 = 8;

/// A species' mesh at level of detail `lod`: 0 every spray of every clump not buried ([`BURIED_LEAST`]), 1 one in ten
/// of them ([`REDUCED_SHARE`]) in larger cards; the ribbons at every or every other clump; at 1 the bark
/// thinner, with fewer sides and no flare.
fn build(model: &Model, lod: usize, light: &CrownLight) -> Mesh {
    let mut mesh = Mesh::new();
    let mut rng =
        Rng(0x9e37_79b9 ^ (model.species as u32).wrapping_mul(2_654_435_761) ^ lod as u32);
    let sprite = sprite_slot(foliage(model.species));
    let height = model.species.height_m();
    for (first, length) in chains(model) {
        let phase = rng.next();
        ribbon(&mut mesh, model, first, length, sprite, phase, lod, light);
    }
    let mut cards: Vec<(f32, Mesh)> = Vec::new();
    for spray in &model.sprays {
        let p = &model.primitives[spray.clump as usize];
        if p.d[0] < BURIED_LEAST || (lod == 1 && spray.keep >= REDUCED_SHARE) {
            continue;
        }
        let mut s = *spray;
        // A card takes the light where it is drawn, a reduced card too: its leaves stand for the full level's about
        // them, its outer end at the crown's rim seeing the rim's sky. (Lit by the spray it grew from, a buried spray's
        // dark reached the crown's outside, and the reduced level drew darker than the full one and its impostor.)
        if lod == 1 {
            // A larger card drawn back toward its clump's middle, into the sprays it stands for, so the crown's
            // outline stays where it was.
            // A low shrub's sprays are already large against it: its cards grow only so far (their leaves then a
            // little smaller).
            let centre = Vec3::new(p.a[0], p.a[1], p.a[2]);
            let growth = REDUCED_GROWTH.min(0.4 / s.length.max(1e-3)).max(1.0);
            s.length *= growth;
            s.width *= growth;
            s.base = s.base.lerp(centre, 0.45) - s.along * s.length * 0.2;
        }
        let mut card = Mesh::new();
        spray_card(&mut card, model, &s, sprite, lod, light);
        cards.push((p.d[0], card));
    }
    // The most open sprays first, the crown's outside, so the GPU's depth test turns away the pixels of those behind
    // them before they are shaded (alpha-tested leaves cannot be sorted for it), and the bark, mostly inside, last.
    cards.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, card) in cards {
        mesh.append(card);
    }
    let mut bark = Mesh::new();
    for limb in &model.limbs {
        if lod == 1 && limb.order >= 2 {
            continue;
        }
        tube(&mut bark, limb, model.species, height, lod, light);
    }
    mesh.append(bark);
    mesh
}

/// The leaf sprites, painted: [`SPRITE_LAYERS`] layers of [`SPRITE_GRID`]² slots of `size / SPRITE_GRID` texels on
/// `size`² RGBA8, every [`Foliage`] in its slot (`sprite_slot`), with `levels` mip levels (largest first): the bytes of
/// each layer's levels. Red is a leaf's brightness (0.4-1.6 over 0.1-1), or under [`TWIG_RED`] a twig's (drawn in the
/// bark's colour), green and blue its tilt across and along the card (0.5 none), alpha its coverage, kept through the
/// levels (the share of texels past one half stays the base level's).
pub fn leaf_sprites(size: u32, levels: u32) -> Vec<Vec<Vec<u8>>> {
    let slot = size / SPRITE_GRID;
    let per_layer = SPRITE_GRID * SPRITE_GRID;
    let painted: Vec<Vec<[f32; 4]>> = Foliage::ALL.iter().map(|f| paint(*f, slot)).collect();
    let mut layers = Vec::new();
    for layer in 0..SPRITE_LAYERS {
        let mut base = vec![[0.5f32, 0.5, 0.5, 0.0]; (size * size) as usize];
        for (index, foliage) in Foliage::ALL.iter().enumerate() {
            let (l, _) = sprite_slot(*foliage);
            if l != layer {
                continue;
            }
            let s = index as u32 % per_layer;
            let ox = (s % SPRITE_GRID) * slot;
            let oy = (s / SPRITE_GRID) * slot;
            for y in 0..slot {
                for x in 0..slot {
                    base[((oy + y) * size + ox + x) as usize] =
                        painted[index][(y * slot + x) as usize];
                }
            }
        }
        // A bare crown's twigs keep their mass down the levels, not their share past one half (`TWIG_CUT`).
        let (twig_layer, _) = sprite_slot(Foliage::Twigs);
        let plain = (twig_layer == layer).then_some(Foliage::Twigs as u32 % per_layer);
        layers.push(sprite_levels(base, size, levels, plain));
    }
    layers
}

/// A sprite layer's levels, each slot's alpha scaled so as many of its texels pass one half as at the base level, but
/// the `plain` slot's, averaged plainly.
fn sprite_levels(base: Vec<[f32; 4]>, size: u32, levels: u32, plain: Option<u32>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let coverage_of = |texels: &[[f32; 4]], side: u32, ox: u32, oy: u32, n: u32, scale: f32| {
        let mut above = 0usize;
        for y in oy..oy + n {
            for x in ox..ox + n {
                if texels[(y * side + x) as usize][3] * scale >= 0.5 {
                    above += 1;
                }
            }
        }
        above as f32 / (n * n) as f32
    };
    let slot = size / SPRITE_GRID;
    let targets: Vec<f32> = (0..SPRITE_GRID * SPRITE_GRID)
        .map(|s| {
            coverage_of(
                &base,
                size,
                (s % SPRITE_GRID) * slot,
                (s / SPRITE_GRID) * slot,
                slot,
                1.0,
            )
        })
        .collect();
    let mut level = base;
    let mut side = size;
    for k in 0..levels {
        if k > 0 {
            let next = side / 2;
            let mut down = vec![[0f32; 4]; (next * next) as usize];
            for y in 0..next {
                for x in 0..next {
                    let mut sum = [0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let t = level[((y * 2 + dy) * side + x * 2 + dx) as usize];
                        for c in 0..3 {
                            sum[c] += t[c] * t[3];
                        }
                        sum[3] += t[3];
                    }
                    let a = sum[3].max(1e-6);
                    down[(y * next + x) as usize] = if sum[3] > 0.0 {
                        [sum[0] / a, sum[1] / a, sum[2] / a, sum[3] / 4.0]
                    } else {
                        [0.5, 0.5, 0.5, 0.0]
                    };
                }
            }
            let n = next / SPRITE_GRID;
            for s in 0..SPRITE_GRID * SPRITE_GRID {
                let (ox, oy) = ((s % SPRITE_GRID) * n, (s / SPRITE_GRID) * n);
                let target = targets[s as usize];
                if target <= 0.0 || n == 0 || plain == Some(s) {
                    continue;
                }
                let (mut low, mut high) = (0.5f32, 4.0f32);
                for _ in 0..16 {
                    let mid = 0.5 * (low + high);
                    if coverage_of(&down, next, ox, oy, n, mid) < target {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                for y in oy..oy + n {
                    for x in ox..ox + n {
                        let t = &mut down[(y * next + x) as usize];
                        t[3] = (t[3] * high).min(1.0);
                    }
                }
            }
            level = down;
            side = next;
        }
        out.push(
            level
                .iter()
                .flat_map(|t| t.map(unorm8))
                .collect::<Vec<u8>>(),
        );
    }
    out
}

/// The red a sprite's twigs are painted under: the draw takes a texel below it for bark, not a leaf.
pub const TWIG_RED: f32 = 0.09;

/// A canvas a sprite is painted on, twice its size a side (anti-aliased by the distance to each shape's edge, then
/// averaged down). Its y grows down: a spray's stem at the bottom middle, its tip at the top.
struct Canvas {
    side: u32,
    texels: Vec<[f32; 4]>,
}

/// A leaf's outline: its half-width over its length (0 at the stalk, 1 at the tip), at most 1.
#[derive(Clone, Copy, PartialEq)]
enum Outline {
    /// Widest below the middle, pointed (beech, lime, birch).
    Ovate,
    /// Widest at the middle (laurel, live oak, the rainforest's with a drip tip).
    Elliptic,
    /// Long and narrow (eucalyptus, olive, bamboo).
    Lance,
    /// Rounded lobes down each side, widest above the middle (oak).
    Lobed,
}

fn outline(shape: Outline, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match shape {
        Outline::Ovate => (std::f32::consts::PI * t.powf(0.72)).sin().powf(0.8),
        Outline::Elliptic => {
            let body = (std::f32::consts::PI * t.powf(0.9)).sin().powf(0.75);
            // A drip tip: the last tenth narrows to a point.
            body * (1.0 - ((t - 0.88) / 0.12).clamp(0.0, 1.0).powf(0.6) * 0.7)
        }
        Outline::Lance => (std::f32::consts::PI * t.powf(0.6)).sin().powf(1.3),
        Outline::Lobed => {
            let envelope = (std::f32::consts::PI * t.powf(1.15)).sin().powf(0.6);
            let lobes = (t * 4.5 * std::f32::consts::PI).sin().abs().powf(0.7);
            envelope * (0.5 + 0.5 * lobes) * (0.15 + 0.85 * (t * 6.0).min(1.0))
        }
    }
}

impl Canvas {
    fn new(side: u32) -> Self {
        Self {
            side,
            texels: vec![[0.5, 0.5, 0.5, 0.0]; (side * side) as usize],
        }
    }

    /// Paints over the box `low`-`high` (texels) where `shape` puts it: each texel's signed distance to the shape's
    /// edge (texels; negative inside) and its shade there (brightness, tilt across, tilt along).
    fn paint(
        &mut self,
        low: [f32; 2],
        high: [f32; 2],
        shape: impl Fn(f32, f32) -> (f32, [f32; 3]),
    ) {
        let x0 = low[0].floor().max(0.0) as u32;
        let y0 = low[1].floor().max(0.0) as u32;
        let x1 = (high[0].ceil().max(0.0) as u32).min(self.side - 1);
        let y1 = (high[1].ceil().max(0.0) as u32).min(self.side - 1);
        if x0 > x1 || y0 > y1 {
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (d, shade) = shape(x as f32 + 0.5, y as f32 + 0.5);
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let t = &mut self.texels[(y * self.side + x) as usize];
                for c in 0..3 {
                    t[c] += (shade[c] - t[c]) * cover;
                }
                t[3] += (1.0 - t[3]) * cover;
            }
        }
    }

    /// A line from `a` to `b`, `w0` wide at `a` and `w1` at `b`, of one shade.
    fn line(&mut self, a: [f32; 2], b: [f32; 2], w0: f32, w1: f32, shade: [f32; 3]) {
        let pad = w0.max(w1) + 2.0;
        let low = [a[0].min(b[0]) - pad, a[1].min(b[1]) - pad];
        let high = [a[0].max(b[0]) + pad, a[1].max(b[1]) + pad];
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-6);
        self.paint(low, high, |x, y| {
            let (px, py) = (x - a[0], y - a[1]);
            let t = ((px * d[0] + py * d[1]) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (px - d[0] * t, py - d[1] * t);
            (
                (qx * qx + qy * qy).sqrt() - 0.5 * (w0 + (w1 - w0) * t),
                shade,
            )
        });
    }

    /// A twig: a line in the bark's red, rounded a little by its shade across it.
    fn twig(&mut self, a: [f32; 2], b: [f32; 2], w0: f32, w1: f32, tone: f32) {
        let pad = w0.max(w1) + 2.0;
        let low = [a[0].min(b[0]) - pad, a[1].min(b[1]) - pad];
        let high = [a[0].max(b[0]) + pad, a[1].max(b[1]) + pad];
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-6);
        let len = len2.sqrt();
        let across = [-d[1] / len, d[0] / len];
        self.paint(low, high, |x, y| {
            let (px, py) = (x - a[0], y - a[1]);
            let t = ((px * d[0] + py * d[1]) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (px - d[0] * t, py - d[1] * t);
            let half = 0.5 * (w0 + (w1 - w0) * t);
            let u = ((qx * across[0] + qy * across[1]) / half.max(0.5)).clamp(-1.0, 1.0);
            let red = TWIG_RED * tone * (0.55 + 0.45 * (1.0 - u * u).sqrt());
            (
                (qx * qx + qy * qy).sqrt() - half,
                [
                    red.min(TWIG_RED - 0.005),
                    0.5 + 0.35 * u * across[0],
                    0.5 + 0.35 * u * across[1],
                ],
            )
        });
    }

    /// A leaf from `base` along `dir` (unit), `length` long and `width` wide at its widest, its outline `shape`, its
    /// brightness `bright` (the red), tilted by `tilt` (across and along the card, -1 to 1) and folded along its
    /// midrib by `fold`, its veins drawn in.
    #[allow(clippy::too_many_arguments)]
    fn leaf(
        &mut self,
        base: [f32; 2],
        dir: [f32; 2],
        length: f32,
        width: f32,
        shape: Outline,
        bright: f32,
        tilt: [f32; 2],
        fold: f32,
    ) {
        let tip = [base[0] + dir[0] * length, base[1] + dir[1] * length];
        let pad = width + 2.0;
        let low = [base[0].min(tip[0]) - pad, base[1].min(tip[1]) - pad];
        let high = [base[0].max(tip[0]) + pad, base[1].max(tip[1]) + pad];
        let side = [-dir[1], dir[0]];
        self.paint(low, high, |x, y| {
            let (px, py) = (x - base[0], y - base[1]);
            let along = px * dir[0] + py * dir[1];
            let across = px * side[0] + py * side[1];
            let t = along / length;
            let half = 0.5 * width * outline(shape, t);
            let d = (across.abs() - half).max(-along).max(along - length);
            let u = (across / half.max(0.5)).clamp(-1.0, 1.0);
            // The midrib a little paler, side veins running out toward the tip, the edge a little darker.
            let midrib = (1.0 - (across.abs() / (width * 0.035).max(0.6))).clamp(0.0, 1.0);
            let vein_phase = (t - u.abs() * 0.35) * 9.0;
            let vein = (1.0 - (vein_phase - vein_phase.round()).abs() * 7.0).clamp(0.0, 1.0)
                * (1.0 - midrib)
                * 0.6;
            let edge = (u.abs() - 0.75).max(0.0) * 0.5;
            let b = bright * (1.0 + 0.1 * midrib - 0.06 * vein - 0.12 * edge + 0.05 * (t - 0.5));
            // Folded along the midrib: each half's normal leans out from it.
            let lean = fold * u.signum() * u.abs().sqrt();
            let g = tilt[0] + lean * side[0];
            let bl = tilt[1] + lean * side[1];
            (
                d,
                [
                    b.clamp(TWIG_RED + 0.02, 1.0),
                    (0.5 + 0.5 * g).clamp(0.0, 1.0),
                    (0.5 + 0.5 * bl).clamp(0.0, 1.0),
                ],
            )
        });
    }

    /// Averaged down by two a side.
    fn finish(self) -> Vec<[f32; 4]> {
        let half = self.side / 2;
        let mut out = vec![[0f32; 4]; (half * half) as usize];
        for y in 0..half {
            for x in 0..half {
                let mut sum = [0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let t = self.texels[((y * 2 + dy) * self.side + x * 2 + dx) as usize];
                    for c in 0..3 {
                        sum[c] += t[c] * t[3];
                    }
                    sum[3] += t[3];
                }
                let a = sum[3].max(1e-6);
                out[(y * half + x) as usize] = if sum[3] > 0.0 {
                    [sum[0] / a, sum[1] / a, sum[2] / a, sum[3] / 4.0]
                } else {
                    [0.5, 0.5, 0.5, 0.0]
                };
            }
        }
        out
    }
}

fn rotate(d: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [d[0] * c - d[1] * s, d[0] * s + d[1] * c]
}

fn normalize2(d: [f32; 2]) -> [f32; 2] {
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-6);
    [d[0] / l, d[1] / l]
}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// A leaf's brightness as the sprite's red: `bright` (about 1) varied at random, 0.4-1.6 over [`TWIG_RED`]-1.
fn leaf_red(rng: &mut Rng, bright: f32) -> f32 {
    ((bright * rng.range(0.8, 1.2) - 0.4) / 1.2).clamp(TWIG_RED + 0.03, 1.0)
}

fn tilt(rng: &mut Rng, most: f32) -> [f32; 2] {
    [rng.range(-most, most), rng.range(-most, most)]
}

/// A twig's path: from `a` to `b` bending by `bow` (a share of its length, to its left), in `n` steps.
fn path(a: [f32; 2], b: [f32; 2], bow: f32, n: usize) -> Vec<[f32; 2]> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let left = [d[1], -d[0]];
    (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            let lift = 4.0 * t * (1.0 - t) * bow;
            [
                a[0] + d[0] * t + left[0] * lift,
                a[1] + d[1] * t + left[1] * lift,
            ]
        })
        .collect()
}

/// A point `t` (0-1) along a path, and its direction there.
fn along_path(path: &[[f32; 2]], t: f32) -> ([f32; 2], [f32; 2]) {
    let n = path.len() - 1;
    let f = (t.clamp(0.0, 1.0) * n as f32).min(n as f32 - 1e-4);
    let k = f.floor() as usize;
    let at = lerp2(path[k], path[k + 1], f - k as f32);
    let dir = normalize2([path[k + 1][0] - path[k][0], path[k + 1][1] - path[k][1]]);
    (at, dir)
}

/// Draws a twig along `path`, tapering from `w0` to `w1`.
fn draw_path(canvas: &mut Canvas, path: &[[f32; 2]], w0: f32, w1: f32, tone: f32) {
    let n = path.len() - 1;
    for k in 0..n {
        let t0 = k as f32 / n as f32;
        let t1 = (k + 1) as f32 / n as f32;
        canvas.twig(
            path[k],
            path[k + 1],
            w0 + (w1 - w0) * t0,
            w0 + (w1 - w0) * t1,
            tone,
        );
    }
}

/// How a branchlet's leaves grow: their outline and size (shares of the sprite), how many a twig, how they stand.
#[derive(Clone, Copy)]
struct Leafing {
    shape: Outline,
    length: f32,
    width: f32,
    /// Leaves a twig of the sprite's height.
    per_twig: f32,
    /// Side twigs off the main one, and twigs off each side twig.
    sides: usize,
    subs: usize,
    /// The angle a leaf stands from its twig (radians), its spread, and how far it droops toward the stem (0-1).
    angle: f32,
    spread: f32,
    droop: f32,
    /// Leaves gathered at the twigs' ends (oak's rosettes): this many more at each end.
    rosette: usize,
    fold: f32,
}

/// A branchlet of leaves: a main twig from the stem (the slot's bottom middle) to near its top, side twigs off it, and
/// leaves alternately along every twig with one at each end.
fn branchlet(canvas: &mut Canvas, rng: &mut Rng, s: f32, leafing: Leafing) {
    let stem = [s * 0.5, s * 0.995];
    let top = [s * rng.range(0.44, 0.56), s * 0.1];
    let main = path(stem, top, rng.range(-0.08, 0.08), 8);
    let mut twigs: Vec<(Vec<[f32; 2]>, f32, f32)> = vec![(main.clone(), s * 0.02, s * 0.007)];
    for k in 0..leafing.sides {
        let t = 0.22 + 0.5 * (k as f32 + rng.range(0.1, 0.9)) / leafing.sides as f32;
        let (at, dir) = along_path(&main, t);
        let flip = if k % 2 == 0 { 1.0 } else { -1.0 };
        let out = normalize2(rotate(dir, flip * rng.range(0.45, 0.85)));
        let reach = s * rng.range(0.32, 0.46) * (1.0 - 0.4 * t);
        let mut end = [at[0] + out[0] * reach, at[1] + out[1] * reach];
        end[0] = end[0].clamp(s * 0.12, s * 0.88);
        end[1] = end[1].max(s * 0.1);
        let side = path(at, end, -flip * rng.range(0.05, 0.15), 5);
        twigs.push((side.clone(), s * 0.012, s * 0.005));
        for j in 0..leafing.subs {
            let u = 0.35 + 0.45 * (j as f32 + rng.range(0.2, 0.8)) / leafing.subs as f32;
            let (at2, dir2) = along_path(&side, u);
            let flip2 = if j % 2 == 0 { -flip } else { flip };
            let out2 = normalize2(rotate(dir2, flip2 * rng.range(0.5, 0.9)));
            let reach2 = s * rng.range(0.14, 0.22);
            let mut end2 = [at2[0] + out2[0] * reach2, at2[1] + out2[1] * reach2];
            end2[0] = end2[0].clamp(s * 0.1, s * 0.9);
            end2[1] = end2[1].max(s * 0.08);
            twigs.push((path(at2, end2, 0.06, 3), s * 0.007, s * 0.004));
        }
    }
    let tone = rng.range(0.8, 1.1);
    for (twig, w0, w1) in &twigs {
        draw_path(canvas, twig, *w0, *w1, tone);
    }
    // The leaves (where each leaves its twig, its way, its length and width, its tone), the inner ones (near the stem) a
    // little darker.
    type Leaf = ([f32; 2], [f32; 2], f32, f32, f32);
    let mut leaves: Vec<Leaf> = Vec::new();
    for (twig, _, _) in &twigs {
        let length = {
            let mut l = 0.0;
            for k in 1..twig.len() {
                l += ((twig[k][0] - twig[k - 1][0]).powi(2)
                    + (twig[k][1] - twig[k - 1][1]).powi(2))
                .sqrt();
            }
            l
        };
        let count = ((leafing.per_twig * length / s).round() as usize).max(1);
        for k in 0..count {
            let t = 0.12 + 0.86 * (k as f32 + rng.range(0.3, 0.7)) / count as f32;
            let (at, dir) = along_path(twig, t);
            let flip = if k % 2 == 0 { 1.0 } else { -1.0 };
            let angle =
                leafing.angle * (1.0 - 0.35 * t) + rng.range(-leafing.spread, leafing.spread);
            let mut d = normalize2(rotate(dir, flip * angle));
            // Drooping: turned toward the stem (down the sprite).
            d = normalize2([d[0], d[1] + leafing.droop * 0.9]);
            let l = s * leafing.length * rng.range(0.8, 1.15) * (1.0 - 0.25 * t);
            let w = s * leafing.width * rng.range(0.85, 1.12) * (1.0 - 0.2 * t);
            let inner = 1.0 - 0.18 * (at[1] / s);
            leaves.push((at, d, l, w, inner));
        }
        // One at the end, along the twig, and a rosette about it where the leaves gather there.
        let (end, dir) = along_path(twig, 1.0);
        leaves.push((end, dir, s * leafing.length, s * leafing.width, 1.0));
        for r in 0..leafing.rosette {
            let angle = (r as f32 / leafing.rosette as f32 - 0.5) * 2.4 + rng.range(-0.2, 0.2);
            let d = normalize2(rotate(dir, angle));
            leaves.push((
                end,
                d,
                s * leafing.length * rng.range(0.75, 1.05),
                s * leafing.width * rng.range(0.85, 1.1),
                0.95,
            ));
        }
    }
    // Painted in random order, so no twig's leaves all lie over another's.
    for k in (1..leaves.len()).rev() {
        let j = (rng.next() * (k + 1) as f32) as usize;
        leaves.swap(k, j.min(k));
    }
    for (at, d, l, w, inner) in leaves {
        // A short stalk to the leaf.
        let stalk = l * 0.12;
        let base = [at[0] + d[0] * stalk, at[1] + d[1] * stalk];
        let tip = [base[0] + d[0] * l, base[1] + d[1] * l];
        if tip[0] < s * 0.02 || tip[0] > s * 0.98 || tip[1] < s * 0.01 || tip[1] > s * 0.995 {
            continue;
        }
        canvas.twig(at, base, w * 0.08, w * 0.06, 1.0);
        let red = leaf_red(rng, inner);
        let t = tilt(rng, 0.6);
        canvas.leaf(base, d, l, w, leafing.shape, red, t, leafing.fold);
    }
}

/// A flat spray of needles (spruce, fir): a main axis, side branchlets alternately at about 50 degrees, their own
/// side shoots, short needles along every one, the youngest (the tips) paler.
fn needle_spray(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    let stem = [s * 0.5, s * 0.995];
    let top = [s * rng.range(0.46, 0.54), s * 0.05];
    let main = path(stem, top, rng.range(-0.05, 0.05), 8);
    let mut shoots: Vec<(Vec<[f32; 2]>, f32, f32)> = vec![(main.clone(), s * 0.016, s * 0.006)];
    let sides = 9;
    for k in 0..sides {
        let t = 0.12 + 0.8 * (k as f32 + rng.range(0.2, 0.8)) / sides as f32;
        let (at, dir) = along_path(&main, t);
        let flip = if k % 2 == 0 { 1.0 } else { -1.0 };
        let out = normalize2(rotate(dir, flip * rng.range(0.8, 1.0)));
        let reach = s * (0.4 * (1.0 - t) + 0.08) * rng.range(0.85, 1.1);
        let end = [
            (at[0] + out[0] * reach).clamp(s * 0.05, s * 0.95),
            (at[1] + out[1] * reach).max(s * 0.04),
        ];
        let shoot = path(at, end, -flip * 0.08, 4);
        for j in 0..3 {
            let u = 0.3 + 0.25 * j as f32;
            let (a2, d2) = along_path(&shoot, u);
            for flip2 in [-1.0, 1.0] {
                let o2 = normalize2(rotate(d2, flip2 * 0.85));
                let r2 = reach * 0.32 * (1.0 - u * 0.6);
                let e2 = [
                    (a2[0] + o2[0] * r2).clamp(s * 0.04, s * 0.96),
                    (a2[1] + o2[1] * r2).max(s * 0.03),
                ];
                shoots.push((vec![a2, e2], s * 0.005, s * 0.003));
            }
        }
        shoots.push((shoot, s * 0.009, s * 0.004));
    }
    let tone = rng.range(0.8, 1.05);
    for (shoot, w0, w1) in &shoots {
        draw_path(canvas, shoot, *w0, *w1, tone);
    }
    // Needles on both sides of every shoot, pointing forward; the tips paler.
    for (shoot, _, _) in &shoots {
        let length: f32 = shoot
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum();
        let count = (length / (s * 0.009)) as usize;
        for k in 0..count {
            let t = (k as f32 + 0.5) / count.max(1) as f32;
            let (at, dir) = along_path(shoot, t);
            for flip in [-1.0, 1.0] {
                let d = normalize2(rotate(dir, flip * rng.range(0.6, 1.0)));
                let l = s * rng.range(0.035, 0.05);
                let tip = [at[0] + d[0] * l, at[1] + d[1] * l];
                if tip[0] < s * 0.01 || tip[0] > s * 0.99 || tip[1] < s * 0.005 {
                    continue;
                }
                let red = leaf_red(rng, 0.85 + 0.3 * t * t);
                let tl = tilt(rng, 0.7);
                canvas.line(
                    at,
                    tip,
                    s * 0.007,
                    s * 0.003,
                    [red, 0.5 + 0.5 * tl[0], 0.5 + 0.5 * tl[1]],
                );
            }
        }
    }
}

/// Pine needles: a short shoot from the stem forking into three or four, each ending in a dense tuft of long needles
/// fanning forward, a bud at its heart.
fn pine_tufts(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    let stem = [s * 0.5, s * 0.995];
    let fork = [s * rng.range(0.46, 0.54), s * 0.7];
    draw_path(canvas, &path(stem, fork, 0.05, 4), s * 0.03, s * 0.022, 0.9);
    let tufts = 4;
    let mut ends = Vec::new();
    for k in 0..tufts {
        let angle = (k as f32 / (tufts - 1) as f32 - 0.5) * 1.5 + rng.range(-0.15, 0.15);
        let dir = rotate([0.0, -1.0], angle);
        let reach = s * rng.range(0.22, 0.32);
        let end = [fork[0] + dir[0] * reach, fork[1] + dir[1] * reach];
        draw_path(
            canvas,
            &path(fork, end, rng.range(-0.1, 0.1), 3),
            s * 0.02,
            s * 0.014,
            0.9,
        );
        ends.push((fork, end, dir));
    }
    for (from, end, dir) in ends {
        // Needles along the shoot's last part and its end, fanning forward.
        for k in 0..70 {
            let t = 0.35 + 0.65 * rng.next().sqrt();
            let at = lerp2(from, end, t);
            let spread = rng.range(-1.1, 1.1) * (0.4 + 0.6 * t);
            let d = normalize2(rotate(dir, spread));
            let l = s * rng.range(0.2, 0.3) * (0.6 + 0.4 * t);
            let tip = [at[0] + d[0] * l, at[1] + d[1] * l];
            if tip[0] < s * 0.01 || tip[0] > s * 0.99 || tip[1] < s * 0.005 || tip[1] > s * 0.99 {
                continue;
            }
            let red = leaf_red(rng, 0.85 + 0.25 * (k % 3) as f32 / 2.0);
            let tl = tilt(rng, 0.7);
            canvas.line(
                at,
                tip,
                s * 0.0065,
                s * 0.0025,
                [red, 0.5 + 0.5 * tl[0], 0.5 + 0.5 * tl[1]],
            );
        }
        canvas.twig(
            end,
            [end[0] + dir[0] * s * 0.03, end[1] + dir[1] * s * 0.03],
            s * 0.022,
            s * 0.008,
            1.3,
        );
    }
}

/// Scale leaves (cypress): flattened sprays branching three times, each branch a cord of overlapping scales.
fn scale_spray(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    let stem = [s * 0.5, s * 0.995];
    let top = [s * rng.range(0.45, 0.55), s * 0.06];
    let mut branches: Vec<([f32; 2], [f32; 2], f32)> = vec![(stem, top, s * 0.024)];
    let mut queue = vec![(stem, top, s * 0.024, 0usize)];
    while let Some((a, b, w, depth)) = queue.pop() {
        if depth >= 3 {
            continue;
        }
        let pieces = 6 - depth;
        for k in 0..pieces {
            let t = 0.15 + 0.75 * (k as f32 + rng.range(0.2, 0.8)) / pieces as f32;
            let at = lerp2(a, b, t);
            let dir = normalize2([b[0] - a[0], b[1] - a[1]]);
            let flip = if k % 2 == 0 { 1.0 } else { -1.0 };
            let out = normalize2(rotate(dir, flip * rng.range(0.5, 0.75)));
            let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let reach = length * (1.0 - t) * rng.range(0.45, 0.6);
            let end = [
                (at[0] + out[0] * reach).clamp(s * 0.04, s * 0.96),
                (at[1] + out[1] * reach).clamp(s * 0.03, s * 0.97),
            ];
            branches.push((at, end, w * 0.62));
            queue.push((at, end, w * 0.62, depth + 1));
        }
    }
    for (a, b, w) in branches {
        // A cord of scales: overlapping short leaves along it, both sides, so its edge is beaded.
        let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let dir = normalize2([b[0] - a[0], b[1] - a[1]]);
        let count = (length / (w * 0.7)).max(1.0) as usize;
        for k in 0..count {
            let t = (k as f32 + 0.5) / count as f32;
            let at = lerp2(a, b, t);
            let width = w * (1.0 - 0.45 * t);
            let red = leaf_red(rng, 0.85 + 0.3 * t);
            let tl = tilt(rng, 0.6);
            canvas.leaf(
                at,
                dir,
                width * 1.6,
                width * 1.25,
                Outline::Ovate,
                red,
                tl,
                0.3,
            );
        }
    }
}

/// A fan palm's leaf: segments radiating from the stalk's top over most of a circle, their tips split and drooping.
fn fan_leaf(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    let hub = [s * 0.5, s * 0.62];
    draw_path(
        canvas,
        &path([s * 0.5, s * 0.995], hub, 0.0, 3),
        s * 0.03,
        s * 0.02,
        1.2,
    );
    let segments = 34;
    for k in 0..segments {
        let a = (k as f32 / (segments - 1) as f32 - 0.5) * 4.2;
        let dir = rotate([0.0, -1.0], a);
        let l = s * rng.range(0.3, 0.37) * (1.0 - 0.15 * (a / 2.1).powi(2));
        // The outer part hangs down at the fan's sides.
        let droop = (a.abs() / 2.1).powi(2) * 0.25;
        let d = normalize2([dir[0], dir[1] + droop]);
        let red = leaf_red(rng, 0.95 + 0.1 * (k % 2) as f32);
        let tl = tilt(rng, 0.4);
        canvas.leaf(
            hub,
            d,
            l,
            s * 0.07,
            Outline::Lance,
            red,
            [tl[0] + dir[0] * 0.3, tl[1]],
            0.6,
        );
    }
}

/// A palm's frond, for a ribbon: the rachis from the base (the slot's bottom) to the tip, long narrow leaflets down
/// both sides angled toward the tip and drooping, gaps between them.
fn frond(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    let from = [s * 0.5, s * 0.995];
    let to = [s * 0.5, s * 0.01];
    draw_path(canvas, &path(from, to, 0.0, 6), s * 0.025, s * 0.006, 1.6);
    let pairs = 46;
    for k in 2..pairs {
        let f = k as f32 / pairs as f32;
        let at = [s * 0.5, from[1] + (to[1] - from[1]) * f];
        for flip in [-1.0, 1.0] {
            let dir = normalize2([flip * 0.75, -0.62 - 0.25 * f]);
            let l = s * 0.48 * (1.0 - 0.8 * f * f) * rng.range(0.85, 1.0);
            let red = leaf_red(rng, 1.0);
            let tl = tilt(rng, 0.4);
            canvas.leaf(
                at,
                dir,
                l,
                s * 0.03 * (1.0 - 0.4 * f),
                Outline::Lance,
                red,
                tl,
                0.7,
            );
        }
    }
}

/// Bare twigs: forking from the stem up through the sprite, each fork thinner and more crooked, the last forks fine
/// and many, so a crown of them reads as a haze of twigs from afar.
fn bare_twigs(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    let mut tips: Vec<([f32; 2], [f32; 2], f32, f32)> =
        vec![([s * 0.5, s * 0.995], [0.0, -1.0], s * 0.26, s * 0.022)];
    for generation in 0..7 {
        let mut next = Vec::new();
        for (at, dir, len, w) in tips {
            let end = [at[0] + dir[0] * len, at[1] + dir[1] * len];
            if end[0] < s * 0.02 || end[0] > s * 0.98 || end[1] < s * 0.01 {
                continue;
            }
            let bend = path(at, end, rng.range(-0.18, 0.18), 3);
            draw_path(canvas, &bend, w, w * 0.75, rng.range(0.85, 1.15));
            if generation == 6 {
                continue;
            }
            let forks = if generation < 3 { 3 } else { 2 };
            for f in 0..forks {
                let angle = (f as f32 - (forks - 1) as f32 * 0.5) * rng.range(0.4, 0.75)
                    + rng.range(-0.25, 0.25);
                // Twigs reach up toward the light.
                let d = normalize2(rotate(dir, angle));
                let d = normalize2([d[0], d[1] - 0.15]);
                next.push((
                    end,
                    d,
                    len * rng.range(0.62, 0.8),
                    (w * 0.7).max(s * 0.0035),
                ));
            }
        }
        tips = next;
    }
}

/// Blades of grass from the bottom of the sprite, arching.
fn grass(canvas: &mut Canvas, rng: &mut Rng, s: f32) {
    for _ in 0..14 {
        let base = [s * rng.range(0.28, 0.72), s * 0.995];
        let lean = rng.range(-0.4, 0.4);
        let height = s * rng.range(0.7, 0.97);
        let red = leaf_red(rng, 1.0);
        let tl = tilt(rng, 0.5);
        let mut previous = base;
        for k in 1..=12 {
            let f = k as f32 / 12.0;
            let at = [base[0] + lean * height * f * f, base[1] - height * f];
            let w0 = s * 0.04 * (1.0 - (f - 1.0 / 12.0)).max(0.05);
            let w1 = s * 0.04 * (1.0 - f).max(0.02);
            canvas.line(
                previous,
                at,
                w0,
                w1,
                [red * (0.8 + 0.25 * f), 0.5 + 0.5 * tl[0], 0.5 + 0.5 * tl[1]],
            );
            previous = at;
        }
    }
}

/// A sprite of `slot` texels a side.
fn paint(foliage: Foliage, slot: u32) -> Vec<[f32; 4]> {
    let side = slot * 2;
    let s = side as f32;
    let mut canvas = Canvas::new(side);
    let mut rng = Rng(0x51ed_270b ^ (foliage as u32 + 1).wrapping_mul(0x2c1b_3c6d));
    let leafing = Leafing {
        shape: Outline::Ovate,
        length: 0.11,
        width: 0.065,
        per_twig: 15.0,
        sides: 4,
        subs: 2,
        angle: 0.95,
        spread: 0.25,
        droop: 0.0,
        rosette: 0,
        fold: 0.35,
    };
    match foliage {
        Foliage::Broadleaf => branchlet(&mut canvas, &mut rng, s, leafing),
        Foliage::Tropical => branchlet(
            &mut canvas,
            &mut rng,
            s,
            Leafing {
                shape: Outline::Elliptic,
                length: 0.25,
                width: 0.11,
                per_twig: 6.0,
                sides: 3,
                subs: 0,
                angle: 0.85,
                spread: 0.3,
                droop: 0.2,
                fold: 0.45,
                ..leafing
            },
        ),
        Foliage::Lobed => branchlet(
            &mut canvas,
            &mut rng,
            s,
            Leafing {
                shape: Outline::Lobed,
                length: 0.13,
                width: 0.085,
                per_twig: 5.0,
                sides: 4,
                subs: 2,
                angle: 0.9,
                rosette: 4,
                fold: 0.25,
                ..leafing
            },
        ),
        Foliage::SmallLeaf => branchlet(
            &mut canvas,
            &mut rng,
            s,
            Leafing {
                shape: Outline::Elliptic,
                length: 0.065,
                width: 0.034,
                per_twig: 26.0,
                sides: 5,
                subs: 3,
                angle: 0.8,
                spread: 0.35,
                fold: 0.3,
                ..leafing
            },
        ),
        Foliage::Lanceolate => branchlet(
            &mut canvas,
            &mut rng,
            s,
            Leafing {
                shape: Outline::Lance,
                length: 0.17,
                width: 0.034,
                per_twig: 14.0,
                sides: 4,
                subs: 1,
                angle: 0.55,
                spread: 0.3,
                droop: 0.35,
                fold: 0.5,
                ..leafing
            },
        ),
        Foliage::Needles => needle_spray(&mut canvas, &mut rng, s),
        Foliage::PineTuft => pine_tufts(&mut canvas, &mut rng, s),
        Foliage::Scale => scale_spray(&mut canvas, &mut rng, s),
        Foliage::Fan => fan_leaf(&mut canvas, &mut rng, s),
        Foliage::Frond => frond(&mut canvas, &mut rng, s),
        Foliage::Twigs => bare_twigs(&mut canvas, &mut rng, s),
        Foliage::Grass => grass(&mut canvas, &mut rng, s),
    }
    canvas.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_species_has_a_mesh_at_each_level() {
        // The crowns' light is the meshes' new cost: timed alone, one species after another, then the meshes whole.
        let start = std::time::Instant::now();
        let lights: Vec<CrownLight> = models().iter().map(CrownLight::new).collect();
        let alone = start.elapsed();
        let start = std::time::Instant::now();
        let meshes = tree_meshes();
        eprintln!(
            "crowns' light {:.1} ms alone ({} grids); the meshes with it {:.1} ms",
            alone.as_secs_f64() * 1e3,
            lights.len(),
            start.elapsed().as_secs_f64() * 1e3
        );
        assert_eq!(meshes.ranges.len(), models().len());
        eprintln!(
            "{} vertices, {} indices: {} bytes",
            meshes.vertices.len(),
            meshes.indices.len(),
            meshes.vertices.len() as u64 * MESH_VERTEX_BYTES + meshes.indices.len() as u64 * 4
        );
        for (model, ranges) in models().iter().zip(&meshes.ranges) {
            let full = ranges[0].index_count / 3;
            let reduced = ranges[1].index_count / 3;
            eprintln!("{:?}: {full} triangles, {reduced} reduced", model.species);
            assert!(
                full > 20 && full < 24_000,
                "{:?}: {full} triangles at full detail",
                model.species
            );
            assert!(
                reduced > 10 && (reduced as f32) < full as f32 * 0.7,
                "{:?}: {reduced} triangles reduced against {full}",
                model.species
            );
            for (lod, range) in ranges.iter().enumerate() {
                let first = range.first_index as usize;
                for &i in &meshes.indices[first..first + range.index_count as usize] {
                    let v = meshes.vertices[(range.base_vertex as u32 + i) as usize];
                    let [x, y, z] = v.position;
                    // Within the model's own reach (a patch of chaparral is wider than tall), its cards a little past it
                    // and, about its lowest clumps and its roots, into the ground.
                    let reach = model.radius * 1.3 + 0.3;
                    assert!(
                        y > -0.5 && y < 1.3 && x.abs() < reach && z.abs() < reach,
                        "{:?}: a vertex out of its model at {:?}",
                        model.species,
                        v.position
                    );
                    assert_eq!(v.normal[3], if lod == 0 { 0 } else { 127 });
                }
            }
        }
    }

    #[test]
    fn the_palm_and_the_grass_grow_ribbons_and_crowns_none() {
        let runs = |species: Species| {
            let model = models().iter().find(|m| m.species == species).unwrap();
            chains(model)
        };
        // 34 fronds (four of them dead), 22 blades, every one found; no crown's lobes taken for one.
        assert_eq!(runs(Species::CoconutPalm).len(), 34);
        assert_eq!(runs(Species::GrassTuft).len(), 22);
        for species in [
            Species::Rainforest,
            Species::Oak,
            Species::Spruce,
            Species::Redwood,
            Species::Cypress,
        ] {
            assert!(runs(species).is_empty(), "{species:?}: {:?}", runs(species));
        }
    }

    #[test]
    fn the_near_shader_tiles_the_reduced_cards_as_built() {
        let shader = include_str!("../../shaders/near_trees.wgsl");
        assert!(shader.contains(&format!("const SPRITE_GRID: f32 = {SPRITE_GRID}.0;")));
        assert!(shader.contains(&format!("const REDUCED_TILES: f32 = {REDUCED_TILES}.0;")));
        // The mark a quarter of the way between two layers' 32s, clear of the rounding either way, and the last layer's
        // marked code within a signed byte.
        const { assert!(TILED_MARK * 4 == 32) };
        const { assert!((SPRITE_LAYERS - 1) * 32 + TILED_MARK <= 127) };
    }

    /// Every slot keeps its share of texels past one half down the levels, but the bare twigs', which keep their mean
    /// coverage (the mass of twigs a far texel holds).
    #[test]
    fn the_sprites_keep_their_coverage_down_the_levels() {
        let layers = leaf_sprites(96 * SPRITE_GRID, 4);
        assert_eq!(layers.len(), SPRITE_LAYERS as usize);
        let (twig_layer, twig_rect) = sprite_slot(Foliage::Twigs);
        // A level's texels in the twigs' slot (or out of it): their share past one half and their mean alpha.
        let stats = |bytes: &[u8], twigs: bool| {
            let side = ((bytes.len() / 4) as f32).sqrt() as usize;
            let (mut past, mut sum, mut n) = (0usize, 0.0f32, 0usize);
            for (i, t) in bytes.chunks(4).enumerate() {
                let (u, v) = (
                    (i % side) as f32 / side as f32,
                    (i / side) as f32 / side as f32,
                );
                let inside =
                    u >= twig_rect[0] && u < twig_rect[2] && v >= twig_rect[1] && v < twig_rect[3];
                if inside == twigs {
                    past += usize::from(t[3] >= 128);
                    sum += t[3] as f32 / 255.0;
                    n += 1;
                }
            }
            (past as f32 / n as f32, sum / n as f32)
        };
        for (layer, levels) in layers.iter().enumerate() {
            assert_eq!(levels.len(), 4);
            assert_eq!(levels[0].len(), (96 * SPRITE_GRID).pow(2) as usize * 4);
            let (base, _) = stats(&levels[0], false);
            assert!(base > 0.08 && base < 0.7, "coverage {base}");
            for level in &levels[1..] {
                let (share, _) = stats(level, false);
                assert!((share - base).abs() < 0.06, "{share} against {base}");
            }
            if layer as u32 == twig_layer {
                let (_, mean) = stats(&levels[0], true);
                assert!(mean > 0.04 && mean < 0.4, "twigs' coverage {mean}");
                for level in &levels[1..] {
                    let (_, m) = stats(level, true);
                    assert!((m - mean).abs() < 0.01, "twigs' {m} against {mean}");
                }
            }
        }
    }
}
