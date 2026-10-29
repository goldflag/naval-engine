//! The infrastructure's meshes: 24-byte vertices ([`InfraVertex`]) in chunk-local metres (x east and z south from the
//! chunk's corner, y metres above the sea), each with its sRGB colour and its [`Material`], and the indices in five
//! [`Group`]s the layer draws by distance and setting: the faces first, then the lines (wires and lattice members
//! opened across the view, [`InfraMesh::lines`]) in the same groups, which the layer blends over the finished scene.

use bytemuck::{Pod, Zeroable};

use super::airfield::RunwaySpan;
use glam::{Vec2, Vec3};
use half::f16;

/// One vertex: chunk-local position, a snorm8 normal (w: the material's parameter, a road deck's half width in metres),
/// the material's coordinates (two f16: metres along and across, or along and up), the sRGB colour and the material.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct InfraVertex {
    pub position: [f32; 3],
    pub normal: [i8; 4],
    pub uv: [u16; 2],
    pub colour: [u8; 4],
}

/// How a face is shaded (`infra.wgsl`'s `MAT_*`, the colour's fourth byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Material {
    /// Painted steel or fibreglass in its colour, a little glossy.
    Paint = 0,
    /// Concrete: rough, its colour mottled a little.
    Concrete = 1,
    /// A container's corrugated side (ribs along `uv.x`, 0.3 m apart) or roof, in its colour.
    Container = 2,
    /// A row of stacked containers seen from afar: each slot's colour picked from the container palette by a hash of
    /// `uv` (metres along the row and up it) and the row's seed (the colour's three bytes).
    Stacks = 3,
    /// A road deck's asphalt: `uv` metres along and across, white edge lines and dashed lanes by its half width (the
    /// normal's w, metres).
    Deck = 4,
    /// Dark glass (a boat's windows).
    Glass = 5,
    /// A hull's gloss paint.
    Hull = 6,
    /// Structural steel and cables, a little metallic.
    Steel = 7,
    /// A runway's half from its end, laid on the ground: `uv` metres along from the threshold and across (the landing
    /// pilot's right positive), the parameter its half width, the colour's bytes the designator's number and letter
    /// (0 none, 1 L, 2 C, 3 R) and flags ([`super::airfield`]). `Runway` to `Apron` are decals.
    Runway = 8,
    /// A taxiway laid on the ground: `uv` metres along (wrapping) and across, the parameter its half width.
    Taxiway = 9,
    /// An apron's concrete laid on the ground, in its colour.
    Apron = 10,
    /// A power line's wire: opened across the view in the vertex shader ([`MeshBuilder::wire`]), blended by a little
    /// more than the share of the pixel it covers so a span stays just seen against the sky. Not a decal.
    Wire = 11,
    /// A lattice's members afar (a tower's legs, bracing and arms, a mast's legs): opened across the view as a wire is
    /// ([`MeshBuilder::member`]), blended by the share of the pixel they cover, so a tower thins to nothing with
    /// distance.
    Lattice = 12,
    /// Crushed coral laid on the ground (a generated base's runway, taxiways, hardstands and roads, `infra::bases`): `uv`
    /// metres along and across, the parameter the paved half width (wider, a strip's shoulders), the colour's bytes its
    /// look (0 a runway, 1 a taxiway, 2 a hardstand, 3 a road), its whole half width (m, the edge that frays into the
    /// ground) and a seed. A decal, as `Runway` to `Apron`.
    Coral = 13,
    /// Pierced steel planking (Marston mat) laid on the ground: `uv` metres along and across, the parameter its half
    /// width, the colour's first byte the share rusted. Planks 15 in by 10 ft across the strip, staggered, their holes
    /// and seams resolved only up close. A decal.
    Marston = 14,
    /// Bulldozed earth (a revetment's berm) in its colour, its tops grown over; the parameter 1 for sandbags (a gun pit's
    /// wall: courses of bags along `uv`, metres along and up).
    Earth = 15,
    /// Corrugated iron (a Quonset hut's skin) in its colour: `uv.x` metres across the corrugation, 76 mm apart.
    Iron = 16,
    /// Tent canvas in its colour, faded where the sun has it.
    Canvas = 17,
    /// Weathered planks in their colour: `uv` metres along and up, boards 20 cm high.
    Wood = 18,
    /// A block of standing fuel drums (55 US gal, 0.58 m across): its top the drums' heads, its sides their columns,
    /// each drum's colour hashed from its place; `uv` metres along and across (the top) or along and up (the sides).
    Drums = 19,
}

impl Material {
    /// Whether a face of this material lies on the ground: pulled toward the eye, casting nothing.
    pub fn is_decal(self) -> bool {
        matches!(
            self,
            Self::Runway | Self::Taxiway | Self::Apron | Self::Coral | Self::Marston
        )
    }
}

/// What a range of indices is, for the layer's choice of what to draw. In this order, so each choice is one range or
/// two: a chunk made far draws `Decal` to `Structure`, one made near `Decal` to `Detail` past the close range and
/// `Decal` with `Structure` to `Near` within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Group {
    /// Laid on the ground (runways): pulled toward the eye, casting nothing.
    Decal = 0,
    /// Stand-ins drawn only where the chunk's close detail is not (container stacks as rows, boats in outline).
    Far = 1,
    /// Structures drawn at every range (bridges).
    Structure = 2,
    /// Detail made only for chunks within the near range (a suspension bridge's hangers).
    Detail = 3,
    /// Detail made only for chunks within the near range and drawn only within the close range, `Far` standing in
    /// past it (containers one by one, boats in full).
    Near = 4,
}

/// Groups a mesh holds.
pub const GROUPS: usize = 5;

/// What a chunk's mesh holds, for the host's status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct InfraCounts {
    pub containers: u32,
    pub boats: u32,
    pub bridges: u32,
    pub runways: u32,
    /// Power towers and poles, and communication masts.
    pub towers: u32,
    /// Generated bases' pieces (`infra::bases`: huts, tents, revetments, pits, towers, dumps, vehicles).
    pub base_pieces: u32,
}

impl std::ops::AddAssign for InfraCounts {
    fn add_assign(&mut self, o: Self) {
        self.containers += o.containers;
        self.boats += o.boats;
        self.bridges += o.bridges;
        self.runways += o.runways;
        self.towers += o.towers;
        self.base_pieces += o.base_pieces;
    }
}

/// A chunk's mesh: its vertices, its indices group after group ([`Group`]: group g's faces are
/// `ranges[g]..ranges[g + 1]`), then its lines group after group (`lines[g]..lines[g + 1]`, from `ranges[GROUPS]`), its
/// bounds (chunk-local) and what it holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InfraMesh {
    pub vertices: Vec<InfraVertex>,
    pub indices: Vec<u32>,
    pub ranges: [u32; GROUPS + 1],
    /// The lines' groups ([`Material::Wire`] and [`Material::Lattice`]), after the faces'.
    pub lines: [u32; GROUPS + 1],
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub counts: InfraCounts,
    /// The runways it draws a part of (chunk-local), for their lights.
    pub runways: Vec<RunwaySpan>,
}

impl InfraMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Group `g`'s faces' indices.
    pub fn range(&self, g: Group) -> std::ops::Range<u32> {
        self.ranges[g as usize]..self.ranges[g as usize + 1]
    }

    /// Group `g`'s lines' indices.
    pub fn line_range(&self, g: Group) -> std::ops::Range<u32> {
        self.lines[g as usize]..self.lines[g as usize + 1]
    }

    /// Bytes on the GPU.
    pub fn bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<InfraVertex>() + self.indices.len() * 4
    }
}

fn f16_bits(v: f32) -> u16 {
    f16::from_f32(v).to_bits()
}

fn snorm(n: Vec3, w: i8) -> [i8; 4] {
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
    [q(n.x), q(n.y), q(n.z), w]
}

/// The look of a face: its sRGB colour, material and the material's parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub colour: [u8; 3],
    pub material: Material,
    pub param: i8,
}

impl Paint {
    pub const fn new(colour: [u8; 3], material: Material) -> Self {
        Self {
            colour,
            material,
            param: 0,
        }
    }
}

/// Faces of an oriented box ([`MeshBuilder::block`]).
pub mod faces {
    pub const TOP: u8 = 1;
    pub const BOTTOM: u8 = 2;
    /// The end facing along +a, and −a.
    pub const END_POS: u8 = 4;
    pub const END_NEG: u8 = 8;
    /// The side facing along +c (a turned a quarter clockwise seen from above), and −c.
    pub const SIDE_POS: u8 = 16;
    pub const SIDE_NEG: u8 = 32;
    pub const ALL: u8 = 63;
    pub const SHELL: u8 = ALL & !BOTTOM;
}

/// An oriented box on the ground: its middle (x, z), the unit direction of its long axis `a` (its across axis `c` is
/// `a` turned a quarter turn: (−a.z, a.x)), half its length and width, its foot and top (m above the sea).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub centre: Vec2,
    pub axis: Vec2,
    pub half_a: f32,
    pub half_c: f32,
    pub y0: f32,
    pub y1: f32,
}

impl Block {
    pub fn across(&self) -> Vec2 {
        Vec2::new(-self.axis.y, self.axis.x)
    }

    /// The four corners at the foot, counter-clockwise seen from above in (x, z) with y up... in the order (−a, −c),
    /// (+a, −c), (+a, +c), (−a, +c).
    pub fn corners(&self) -> [Vec2; 4] {
        let a = self.axis * self.half_a;
        let c = self.across() * self.half_c;
        [
            self.centre - a - c,
            self.centre + a - c,
            self.centre + a + c,
            self.centre - a + c,
        ]
    }
}

/// Accumulates faces into an [`InfraMesh`], into the [`Group`] chosen.
pub struct MeshBuilder {
    vertices: Vec<InfraVertex>,
    groups: [Vec<u32>; GROUPS],
    /// The lines' triangles by group.
    lines: [Vec<u32>; GROUPS],
    group: Group,
    min: Vec3,
    max: Vec3,
    near: bool,
    pub counts: InfraCounts,
    pub runways: Vec<RunwaySpan>,
}

impl Default for MeshBuilder {
    fn default() -> Self {
        Self::new(true)
    }
}

impl MeshBuilder {
    /// A builder for a chunk within the near range (`near`: its [`Group::Detail`] and [`Group::Near`] made) or beyond it.
    pub fn new(near: bool) -> Self {
        Self {
            vertices: Vec::new(),
            groups: Default::default(),
            lines: Default::default(),
            group: Group::Structure,
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
            near,
            counts: InfraCounts::default(),
            runways: Vec::new(),
        }
    }

    /// Whether the chunk's near detail is made: the makers of [`Group::Detail`] and [`Group::Near`] faces skip them
    /// when not.
    pub fn near(&self) -> bool {
        self.near
    }

    /// The group the next faces go into.
    pub fn set_group(&mut self, group: Group) {
        self.group = group;
    }

    pub fn group(&self) -> Group {
        self.group
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    fn push(&mut self, p: Vec3, n: Vec3, uv: Vec2, paint: Paint) -> u32 {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
        let i = self.vertices.len() as u32;
        let [r, g, b] = paint.colour;
        self.vertices.push(InfraVertex {
            position: p.to_array(),
            normal: snorm(n, paint.param),
            uv: [f16_bits(uv.x), f16_bits(uv.y)],
            colour: [r, g, b, paint.material as u8],
        });
        i
    }

    fn triangle(&mut self, t: [u32; 3]) {
        self.groups[self.group as usize].extend_from_slice(&t);
    }

    /// A planar face from its points in order (counter-clockwise seen from the side its normal faces, which is
    /// `outward`'s side; a face wound the other way is flipped), fanned from its first point, each with its `uv`.
    pub fn face(&mut self, points: &[Vec3], uv: &[Vec2], outward: Vec3, paint: Paint) {
        if points.len() < 3 {
            return;
        }
        let mut n = (points[1] - points[0])
            .cross(points[2] - points[0])
            .normalize_or_zero();
        let flip = n.dot(outward) < 0.0;
        if flip {
            n = -n;
        }
        if n == Vec3::ZERO {
            n = outward.normalize_or(Vec3::Y);
        }
        let first = self.vertices.len() as u32;
        for (p, t) in points.iter().zip(uv) {
            self.push(*p, n, *t, paint);
        }
        for k in 1..points.len() as u32 - 1 {
            if flip {
                self.triangle([first, first + k + 1, first + k]);
            } else {
                self.triangle([first, first + k, first + k + 1]);
            }
        }
    }

    /// A quad from four points counter-clockwise seen from `outward`'s side.
    pub fn quad(&mut self, p: [Vec3; 4], uv: [Vec2; 4], outward: Vec3, paint: Paint) {
        self.face(&p, &uv, outward, paint);
    }

    /// A smooth strip between two rows of points (`left` and `right`, the same count), each point its own normal: a
    /// deck's top along its line, say. `uv` per pair. The strip faces up (or `outward`).
    pub fn strip(
        &mut self,
        left: &[Vec3],
        right: &[Vec3],
        uv_left: &[Vec2],
        uv_right: &[Vec2],
        normals: &[Vec3],
        paint: Paint,
    ) {
        let n = left.len().min(right.len());
        if n < 2 {
            return;
        }
        let first = self.vertices.len() as u32;
        for k in 0..n {
            self.push(left[k], normals[k], uv_left[k], paint);
            self.push(right[k], normals[k], uv_right[k], paint);
        }
        for k in 0..n as u32 - 1 {
            let (l0, r0, l1, r1) = (
                first + 2 * k,
                first + 2 * k + 1,
                first + 2 * k + 2,
                first + 2 * k + 3,
            );
            // Seen from above with x east and z south, left of the direction of travel is −c: wound so the top faces
            // up whichever way the line runs (checked by the normal).
            let a = Vec3::from(self.vertices[l0 as usize].position);
            let b = Vec3::from(self.vertices[r0 as usize].position);
            let c = Vec3::from(self.vertices[l1 as usize].position);
            let up = (b - a).cross(c - a).dot(normals[k as usize]) >= 0.0;
            if up {
                self.triangle([l0, r0, l1]);
                self.triangle([l1, r0, r1]);
            } else {
                self.triangle([l0, l1, r0]);
                self.triangle([l1, r1, r0]);
            }
        }
    }

    /// An oriented box's chosen `faces` (the [`faces`] bits). The sides' `uv`: metres along the face and up from its
    /// foot; the top's: metres along a and across c from the box's −a, −c corner.
    pub fn block(&mut self, b: &Block, which: u8, paint: Paint) {
        let [c0, c1, c2, c3] = b.corners();
        let lo = |p: Vec2| Vec3::new(p.x, b.y0, p.y);
        let hi = |p: Vec2| Vec3::new(p.x, b.y1, p.y);
        let a3 = Vec3::new(b.axis.x, 0.0, b.axis.y);
        let c = b.across();
        let c3v = Vec3::new(c.x, 0.0, c.y);
        let (la, lc, h) = (2.0 * b.half_a, 2.0 * b.half_c, b.y1 - b.y0);
        if which & faces::TOP != 0 {
            self.quad(
                [hi(c0), hi(c1), hi(c2), hi(c3)],
                [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(la, 0.0),
                    Vec2::new(la, lc),
                    Vec2::new(0.0, lc),
                ],
                Vec3::Y,
                paint,
            );
        }
        if which & faces::BOTTOM != 0 {
            self.quad(
                [lo(c0), lo(c3), lo(c2), lo(c1)],
                [Vec2::ZERO; 4],
                -Vec3::Y,
                paint,
            );
        }
        let side = |p: Vec2, q: Vec2, out: Vec3, len: f32, this: &mut Self| {
            this.quad(
                [lo(p), lo(q), hi(q), hi(p)],
                [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(len, 0.0),
                    Vec2::new(len, h),
                    Vec2::new(0.0, h),
                ],
                out,
                paint,
            );
        };
        if which & faces::SIDE_NEG != 0 {
            side(c0, c1, -c3v, la, self);
        }
        if which & faces::END_POS != 0 {
            side(c1, c2, a3, lc, self);
        }
        if which & faces::SIDE_POS != 0 {
            side(c2, c3, c3v, la, self);
        }
        if which & faces::END_NEG != 0 {
            side(c3, c0, -a3, lc, self);
        }
    }

    /// A wire through `points` ([`Material::Wire`], the parameter its radius in centimetres): a line
    /// ([`Self::line`]).
    pub fn wire(&mut self, points: &[Vec3], radius_cm: i8, colour: [u8; 3]) {
        self.line(points, radius_cm, colour, Material::Wire);
    }

    /// A lattice's member afar through `points` ([`Material::Lattice`], the parameter its radius in centimetres, the
    /// width it stands for): a line ([`Self::line`]).
    pub fn member(&mut self, points: &[Vec3], radius_cm: i8, colour: [u8; 3]) {
        self.line(points, radius_cm, colour, Material::Lattice);
    }

    /// A line through `points`: each segment a ribbon of two coincident edges the vertex shader opens across the view
    /// (the normal the line's direction, `uv.y` the side, the parameter the radius in centimetres), at least about a
    /// pixel wide however far, and blended by how much of that it covers. Into the lines' groups, drawn unculled.
    fn line(&mut self, points: &[Vec3], radius_cm: i8, colour: [u8; 3], material: Material) {
        let paint = Paint {
            colour,
            material,
            param: radius_cm,
        };
        let mut along = 0.0;
        for w in points.windows(2) {
            let d = (w[1] - w[0]).normalize_or(Vec3::X);
            let length = w[0].distance(w[1]);
            let at =
                |p: Vec3, s: f32, side: f32, b: &mut Self| b.push(p, d, Vec2::new(s, side), paint);
            let a0 = at(w[0], along, -1.0, self);
            let a1 = at(w[0], along, 1.0, self);
            let b1 = at(w[1], along + length, 1.0, self);
            let b0 = at(w[1], along + length, -1.0, self);
            self.lines[self.group as usize].extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
            along = (along + length) % 512.0;
        }
    }

    /// A square beam from `a` to `b`, `half` its half width, its sides turned so one faces up as far as it can (a
    /// cable's, a leg's, a mast's): four faces, no ends.
    pub fn beam(&mut self, a: Vec3, b: Vec3, half: f32, paint: Paint) {
        let d = b - a;
        let len = d.length();
        if len < 1e-4 {
            return;
        }
        let dir = d / len;
        let helper = if dir.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
        let u = dir.cross(helper).normalize() * half;
        let v = dir.cross(u).normalize() * half;
        let corners = [u + v, -u + v, -u - v, u - v];
        for k in 0..4 {
            let (p, q) = (corners[k], corners[(k + 1) % 4]);
            let out = (p + q).normalize_or_zero();
            self.quad(
                [a + p, a + q, b + q, b + p],
                [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(half * 2.0, 0.0),
                    Vec2::new(half * 2.0, len),
                    Vec2::new(0.0, len),
                ],
                out,
                paint,
            );
        }
    }

    /// A prism: `ring` (counter-clockwise seen from above in x and z... any winding) from `y0` to `y1`, its walls
    /// flat-shaded and its top fanned (convex rings only; a ring of three to eight points).
    pub fn prism(&mut self, ring: &[Vec2], y0: f32, y1: f32, top: bool, paint: Paint) {
        let n = ring.len();
        if n < 3 {
            return;
        }
        let centre = ring.iter().copied().sum::<Vec2>() / n as f32;
        for k in 0..n {
            let (p, q) = (ring[k], ring[(k + 1) % n]);
            let mid = (p + q) * 0.5 - centre;
            let out = Vec3::new(mid.x, 0.0, mid.y);
            let len = p.distance(q);
            self.quad(
                [
                    Vec3::new(p.x, y0, p.y),
                    Vec3::new(q.x, y0, q.y),
                    Vec3::new(q.x, y1, q.y),
                    Vec3::new(p.x, y1, p.y),
                ],
                [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(len, 0.0),
                    Vec2::new(len, y1 - y0),
                    Vec2::new(0.0, y1 - y0),
                ],
                out,
                paint,
            );
        }
        if top {
            let points: Vec<Vec3> = ring.iter().map(|p| Vec3::new(p.x, y1, p.y)).collect();
            let uv: Vec<Vec2> = ring.iter().map(|p| *p - centre).collect();
            self.face(&points, &uv, Vec3::Y, paint);
        }
    }

    /// A loft between two rings of the same count (a hull from its keel's outline to its deck's): each quad its own
    /// face, outward from the axis through `centre`.
    pub fn loft(&mut self, low: &[Vec3], high: &[Vec3], centre: Vec3, paint: Paint) {
        let n = low.len().min(high.len());
        for k in 0..n {
            let j = (k + 1) % n;
            let mid = (low[k] + low[j] + high[k] + high[j]) * 0.25 - centre;
            let out = Vec3::new(mid.x, 0.0, mid.z).normalize_or(Vec3::Y);
            let len = low[k].distance(low[j]);
            let h = low[k].distance(high[k]);
            self.quad(
                [low[k], low[j], high[j], high[k]],
                [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(len, 0.0),
                    Vec2::new(len, h),
                    Vec2::new(0.0, h),
                ],
                out,
                paint,
            );
        }
    }

    /// The mesh, its faces' groups in order, then its lines'.
    pub fn finish(self) -> InfraMesh {
        let mut indices =
            Vec::with_capacity(self.groups.iter().chain(&self.lines).map(Vec::len).sum());
        let mut ranges = [0u32; GROUPS + 1];
        for (g, list) in self.groups.iter().enumerate() {
            ranges[g] = indices.len() as u32;
            indices.extend_from_slice(list);
        }
        ranges[GROUPS] = indices.len() as u32;
        let mut lines = [0u32; GROUPS + 1];
        for (g, list) in self.lines.iter().enumerate() {
            lines[g] = indices.len() as u32;
            indices.extend_from_slice(list);
        }
        lines[GROUPS] = indices.len() as u32;
        let empty = self.vertices.is_empty();
        InfraMesh {
            vertices: self.vertices,
            indices,
            ranges,
            lines,
            min: if empty { [0.0; 3] } else { self.min.to_array() },
            max: if empty { [0.0; 3] } else { self.max.to_array() },
            counts: self.counts,
            runways: self.runways,
        }
    }
}

/// A hash of three integers to 0-1 (the buildings' `hash01`, so places hash alike).
pub fn hash01(x: i64, y: i64, seed: i64) -> f32 {
    crate::buildings::hash01(x, y, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle wound counter-clockwise about its vertices' normal (front-facing from outside).
    fn wound_out(mesh: &InfraMesh) -> bool {
        mesh.indices.chunks(3).all(|t| {
            let p = |i: u32| Vec3::from(mesh.vertices[i as usize].position);
            let n = mesh.vertices[t[0] as usize].normal;
            let n = Vec3::new(f32::from(n[0]), f32::from(n[1]), f32::from(n[2]));
            (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).dot(n) > 0.0
        })
    }

    #[test]
    fn a_block_is_closed_and_faces_out() {
        let mut m = MeshBuilder::default();
        let b = Block {
            centre: Vec2::new(10.0, 20.0),
            axis: Vec2::new(0.6, 0.8),
            half_a: 6.0,
            half_c: 1.2,
            y0: 1.0,
            y1: 3.6,
        };
        m.block(
            &b,
            faces::ALL,
            Paint::new([200, 40, 30], Material::Container),
        );
        let mesh = m.finish();
        assert_eq!(mesh.triangles(), 12);
        assert!(wound_out(&mesh));
        // Each face's normal points away from the box's middle.
        let middle = Vec3::new(10.0, 2.3, 20.0);
        for t in mesh.indices.chunks(3) {
            let v = mesh.vertices[t[0] as usize];
            let n = Vec3::new(
                f32::from(v.normal[0]),
                f32::from(v.normal[1]),
                f32::from(v.normal[2]),
            );
            assert!((Vec3::from(v.position) - middle).dot(n) > 0.0);
        }
        assert_eq!((mesh.min[1], mesh.max[1]), (1.0, 3.6));
        assert!((mesh.min[0] - (10.0 - 6.0 * 0.6 - 1.2 * 0.8)).abs() < 1e-4);
        assert_eq!(
            mesh.vertices[0].colour,
            [200, 40, 30, Material::Container as u8]
        );
    }

    #[test]
    fn groups_come_out_in_order() {
        let mut m = MeshBuilder::default();
        let b = Block {
            centre: Vec2::ZERO,
            axis: Vec2::X,
            half_a: 1.0,
            half_c: 1.0,
            y0: 0.0,
            y1: 1.0,
        };
        m.set_group(Group::Near);
        m.block(&b, faces::TOP, Paint::new([1, 2, 3], Material::Paint));
        m.set_group(Group::Structure);
        m.block(
            &b,
            faces::TOP | faces::SIDE_POS,
            Paint::new([1, 2, 3], Material::Paint),
        );
        let mesh = m.finish();
        assert_eq!(mesh.range(Group::Decal), 0..0);
        assert_eq!(mesh.range(Group::Far), 0..0);
        assert_eq!(mesh.range(Group::Structure), 0..12);
        assert_eq!(mesh.range(Group::Detail), 12..12);
        assert_eq!(mesh.range(Group::Near), 12..18);
        assert!(wound_out(&mesh));
    }

    #[test]
    fn beams_prisms_and_lofts_face_out() {
        let mut m = MeshBuilder::default();
        let paint = Paint::new([90, 90, 90], Material::Steel);
        m.beam(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            0.5,
            paint,
        );
        m.beam(
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(30.0, 8.0, 4.0),
            0.3,
            paint,
        );
        let ring: Vec<Vec2> = (0..6)
            .map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 6.0;
                Vec2::new(a.cos(), a.sin()) * 3.0
            })
            .collect();
        m.prism(&ring, 0.0, 4.0, true, paint);
        let low: Vec<Vec3> = ring
            .iter()
            .map(|p| Vec3::new(p.x * 0.7, -1.0, p.y * 0.7))
            .collect();
        let high: Vec<Vec3> = ring.iter().map(|p| Vec3::new(p.x, 1.0, p.y)).collect();
        m.loft(&low, &high, Vec3::ZERO, paint);
        let mesh = m.finish();
        assert_eq!(mesh.triangles(), 8 + 8 + 12 + 4 + 12);
        assert!(wound_out(&mesh));
    }
}
