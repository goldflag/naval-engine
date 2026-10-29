//! Mapbox Vector Tiles (the MVT 2.1 specification): the protobuf read by hand (no generated code: four messages and
//! the wire format's varints, fixed widths and length-delimited fields) and the geometry's command stream decoded into
//! points, lines and polygons in the tile's own integer coordinates (0 to the layer's `extent`, usually 4096, x east and
//! y south; a tile's buffer reaches a little past either end).

use std::fmt;

/// A decoded tile: its layers, in the order the tile holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub layers: Vec<Layer>,
}

impl Tile {
    /// The layer named `name`, if the tile holds one.
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }
}

/// A layer: its features and the key and value tables their tags index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub version: u32,
    pub name: String,
    /// Units across the tile (the specification's default 4096).
    pub extent: u32,
    pub keys: Vec<String>,
    pub values: Vec<Value>,
    pub features: Vec<Feature>,
}

impl Layer {
    /// Feature `f`'s value for `key`.
    pub fn get<'a>(&'a self, f: &Feature, key: &str) -> Option<&'a Value> {
        f.tags.iter().find_map(|&(k, v)| {
            (self.keys.get(k as usize).map(String::as_str) == Some(key))
                .then(|| self.values.get(v as usize))
                .flatten()
        })
    }

    /// Feature `f`'s value for `key` as a string (none when absent or not a string).
    pub fn str<'a>(&'a self, f: &Feature, key: &str) -> Option<&'a str> {
        self.get(f, key).and_then(Value::as_str)
    }

    /// Feature `f`'s value for `key` as a number (an integer, a float or a boolean).
    pub fn number(&self, f: &Feature, key: &str) -> Option<f64> {
        self.get(f, key).and_then(Value::as_f64)
    }

    /// Feature `f`'s tags as (key, value) pairs.
    pub fn tags<'a>(&'a self, f: &'a Feature) -> impl Iterator<Item = (&'a str, &'a Value)> + 'a {
        f.tags.iter().filter_map(|&(k, v)| {
            Some((
                self.keys.get(k as usize)?.as_str(),
                self.values.get(v as usize)?,
            ))
        })
    }
}

/// A tag's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Float(f32),
    Double(f64),
    Int(i64),
    Uint(u64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Float(v) => Some(f64::from(v)),
            Self::Double(v) => Some(v),
            Self::Int(v) => Some(v as f64),
            Self::Uint(v) => Some(v as f64),
            Self::Bool(v) => Some(f64::from(u8::from(v))),
            Self::String(_) => None,
        }
    }
}

/// The specification's geometry types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeomType {
    #[default]
    Unknown,
    Point,
    LineString,
    Polygon,
}

/// A feature: its id, its tags (indices into the layer's keys and values) and its geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feature {
    pub id: Option<u64>,
    pub tags: Vec<(u32, u32)>,
    pub kind: GeomType,
    pub geometry: Geometry,
}

/// A point in the tile's units.
pub type Point = [i32; 2];

/// A polygon: its exterior ring and its holes, each closed implicitly (the last point is not the first repeated).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polygon<P = Point> {
    pub exterior: Vec<P>,
    pub holes: Vec<Vec<P>>,
}

/// A feature's shape.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Geometry<P = Point> {
    #[default]
    Empty,
    Points(Vec<P>),
    Lines(Vec<Vec<P>>),
    Polygons(Vec<Polygon<P>>),
}

impl<P> Geometry<P> {
    /// Every point of the shape, rings and lines in order.
    pub fn points(&self) -> Box<dyn Iterator<Item = &P> + '_> {
        match self {
            Self::Empty => Box::new(std::iter::empty()),
            Self::Points(p) => Box::new(p.iter()),
            Self::Lines(l) => Box::new(l.iter().flatten()),
            Self::Polygons(polys) => Box::new(
                polys
                    .iter()
                    .flat_map(|p| p.exterior.iter().chain(p.holes.iter().flatten())),
            ),
        }
    }

    /// The same shape with every point mapped by `f`.
    pub fn map<Q>(&self, f: &mut impl FnMut(&P) -> Q) -> Geometry<Q> {
        let ring = |r: &Vec<P>, f: &mut dyn FnMut(&P) -> Q| r.iter().map(f).collect::<Vec<Q>>();
        match self {
            Self::Empty => Geometry::Empty,
            Self::Points(p) => Geometry::Points(p.iter().map(&mut *f).collect()),
            Self::Lines(lines) => Geometry::Lines(lines.iter().map(|l| ring(l, f)).collect()),
            Self::Polygons(polys) => Geometry::Polygons(
                polys
                    .iter()
                    .map(|p| Polygon {
                        exterior: ring(&p.exterior, f),
                        holes: p.holes.iter().map(|h| ring(h, f)).collect(),
                    })
                    .collect(),
            ),
        }
    }
}

/// Why a tile did not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes end inside a field.
    Truncated,
    /// A wire type the format does not have (or a group, which MVT never uses).
    WireType(u8),
    /// A varint longer than ten bytes.
    Varint,
    /// A string that is not UTF-8.
    Utf8,
    /// A gzipped tile that would not inflate.
    Gzip(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "the tile ends inside a field"),
            Self::WireType(t) => write!(f, "wire type {t} is not protobuf's"),
            Self::Varint => write!(f, "a varint longer than ten bytes"),
            Self::Utf8 => write!(f, "a string that is not UTF-8"),
            Self::Gzip(why) => write!(f, "gzip: {why}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// The most a gzipped tile may inflate to (a dense city's zoom-14 tile is about a megabyte).
pub const MAX_INFLATED_BYTES: usize = 32 << 20;

/// Decodes a tile's bytes (inflated first when they are gzipped, as some servers send them).
pub fn decode(bytes: &[u8]) -> Result<Tile, DecodeError> {
    decode_layers(bytes, None)
}

/// Decodes only the layers named in `wanted` (every layer when none): a region bake reads a tile's `water` and skips
/// its roads and buildings, which hold most of a coastal tile's bytes. The layers left out are still read far enough to
/// learn their names, so a malformed one is refused all the same.
pub fn decode_layers(bytes: &[u8], wanted: Option<&[&str]>) -> Result<Tile, DecodeError> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let inflated = gunzip(bytes)?;
        return decode_raw(&inflated, wanted);
    }
    decode_raw(bytes, wanted)
}

fn decode_raw(bytes: &[u8], wanted: Option<&[&str]>) -> Result<Tile, DecodeError> {
    let mut tile = Tile::default();
    let mut r = Reader::new(bytes);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (3, 2) => {
                if let Some(l) = layer(r.bytes()?, wanted)? {
                    tile.layers.push(l);
                }
            }
            _ => r.skip(wire)?,
        }
    }
    Ok(tile)
}

/// A layer, or none when `wanted` leaves it out (its features not decoded).
fn layer(bytes: &[u8], wanted: Option<&[&str]>) -> Result<Option<Layer>, DecodeError> {
    let mut l = Layer {
        version: 1,
        extent: 4096,
        ..Layer::default()
    };
    let mut features = Vec::new();
    let mut r = Reader::new(bytes);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (15, 0) => l.version = r.varint()? as u32,
            (1, 2) => l.name = r.string()?,
            // Features are decoded once the whole layer is read (their geometry needs no table, but keep one pass).
            (2, 2) => features.push(r.bytes()?),
            (3, 2) => l.keys.push(r.string()?),
            (4, 2) => l.values.push(value(r.bytes()?)?),
            (5, 0) => l.extent = r.varint()? as u32,
            _ => r.skip(wire)?,
        }
    }
    if wanted.is_some_and(|names| !names.contains(&l.name.as_str())) {
        return Ok(None);
    }
    l.features = features
        .into_iter()
        .map(feature)
        .collect::<Result<_, _>>()?;
    Ok(Some(l))
}

fn value(bytes: &[u8]) -> Result<Value, DecodeError> {
    let mut r = Reader::new(bytes);
    let mut v = Value::Bool(false);
    while let Some((field, wire)) = r.key()? {
        v = match (field, wire) {
            (1, 2) => Value::String(r.string()?),
            (2, 5) => Value::Float(f32::from_bits(r.fixed32()?)),
            (3, 1) => Value::Double(f64::from_bits(r.fixed64()?)),
            (4, 0) => Value::Int(r.varint()? as i64),
            (5, 0) => Value::Uint(r.varint()?),
            (6, 0) => {
                let raw = r.varint()?;
                Value::Int(((raw >> 1) as i64) ^ -((raw & 1) as i64))
            }
            (7, 0) => Value::Bool(r.varint()? != 0),
            _ => {
                r.skip(wire)?;
                continue;
            }
        };
    }
    Ok(v)
}

fn feature(bytes: &[u8]) -> Result<Feature, DecodeError> {
    let mut f = Feature::default();
    let mut tags = Vec::new();
    let mut commands = Vec::new();
    let mut r = Reader::new(bytes);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (1, 0) => f.id = Some(r.varint()?),
            (2, 2) => r.packed(&mut tags)?,
            (2, 0) => tags.push(r.varint()? as u32),
            (3, 0) => {
                f.kind = match r.varint()? {
                    1 => GeomType::Point,
                    2 => GeomType::LineString,
                    3 => GeomType::Polygon,
                    _ => GeomType::Unknown,
                }
            }
            (4, 2) => r.packed(&mut commands)?,
            (4, 0) => commands.push(r.varint()? as u32),
            _ => r.skip(wire)?,
        }
    }
    f.tags = tags
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[k, v]| (k, v))
        .collect();
    f.geometry = geometry(f.kind, &commands);
    Ok(f)
}

/// The command stream's paths: each `MoveTo` starts one, `LineTo`s extend it, `ClosePath` ends a ring. A malformed
/// stream (a count running past its parameters) ends at what was read whole.
pub fn paths(commands: &[u32]) -> (Vec<Vec<Point>>, Vec<bool>) {
    let mut paths: Vec<Vec<Point>> = Vec::new();
    let mut closed = Vec::new();
    let (mut x, mut y) = (0i32, 0i32);
    let mut k = 0;
    while k < commands.len() {
        let header = commands[k];
        k += 1;
        let (id, count) = (header & 7, (header >> 3) as usize);
        match id {
            1 | 2 => {
                if k + 2 * count > commands.len() {
                    break;
                }
                for n in 0..count {
                    x = x.wrapping_add(zigzag(commands[k + 2 * n]));
                    y = y.wrapping_add(zigzag(commands[k + 2 * n + 1]));
                    if id == 1 {
                        paths.push(vec![[x, y]]);
                        closed.push(false);
                    } else if let Some(path) = paths.last_mut() {
                        path.push([x, y]);
                    }
                }
                k += 2 * count;
            }
            7 => {
                if let Some(c) = closed.last_mut() {
                    *c = true;
                }
            }
            _ => break,
        }
    }
    (paths, closed)
}

fn zigzag(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

/// Twice a ring's signed area in the tile's units (x east, y south): positive for the specification's exterior rings
/// (clockwise as drawn, y down), negative for its holes.
pub fn ring_area2(ring: &[Point]) -> i128 {
    // Each term fits an i64 doubled; their sum over a ring of any length and coordinates fits an i128.
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            i128::from(a[0]) * i128::from(b[1]) - i128::from(b[0]) * i128::from(a[1])
        })
        .sum()
}

fn geometry(kind: GeomType, commands: &[u32]) -> Geometry {
    let (paths, _) = paths(commands);
    match kind {
        GeomType::Unknown => Geometry::Empty,
        GeomType::Point => Geometry::Points(paths.into_iter().flatten().collect()),
        GeomType::LineString => {
            Geometry::Lines(paths.into_iter().filter(|p| p.len() >= 2).collect())
        }
        GeomType::Polygon => {
            // Each exterior ring (positive area) starts a polygon; the holes (negative) that follow are its. A ring of
            // no area is dropped.
            let mut polys: Vec<Polygon> = Vec::new();
            for ring in paths.into_iter().filter(|r| r.len() >= 3) {
                let area = ring_area2(&ring);
                if area > 0 {
                    polys.push(Polygon {
                        exterior: ring,
                        holes: Vec::new(),
                    });
                } else if area < 0
                    && let Some(p) = polys.last_mut()
                {
                    p.holes.push(ring);
                }
            }
            Geometry::Polygons(polys)
        }
    }
}

/// Inflates a gzip member (RFC 1952: the header, its optional fields, the DEFLATE stream).
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let bad = |why: &str| DecodeError::Gzip(why.into());
    if bytes.len() < 18 || bytes[2] != 8 {
        return Err(bad("not a DEFLATE member"));
    }
    let flags = bytes[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let len = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
        at += 2 + len;
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            while at < bytes.len() && bytes[at] != 0 {
                at += 1;
            }
            at += 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    let body = bytes
        .get(at..)
        .ok_or_else(|| bad("the header runs past the end"))?;
    // Bounded: a small body that inflates without end is refused, not allocated.
    miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_INFLATED_BYTES)
        .map_err(|e| bad(&format!("{:?}", e.status)))
}

/// The wire format.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn varint(&mut self) -> Result<u64, DecodeError> {
        let mut v = 0u64;
        for shift in (0..70).step_by(7) {
            let b = *self.bytes.get(self.at).ok_or(DecodeError::Truncated)?;
            self.at += 1;
            if shift < 64 {
                v |= u64::from(b & 0x7f) << shift;
            }
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(DecodeError::Varint)
    }

    /// The next field's number and wire type; none at the end.
    fn key(&mut self) -> Result<Option<(u32, u8)>, DecodeError> {
        if self.at >= self.bytes.len() {
            return Ok(None);
        }
        let k = self.varint()?;
        Ok(Some(((k >> 3) as u32, (k & 7) as u8)))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.bytes.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(s)
    }

    fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.varint()? as usize;
        self.take(n)
    }

    fn string(&mut self) -> Result<String, DecodeError> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_owned)
            .map_err(|_| DecodeError::Utf8)
    }

    fn fixed32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn fixed64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn packed(&mut self, out: &mut Vec<u32>) -> Result<(), DecodeError> {
        let mut inner = Reader::new(self.bytes()?);
        while inner.at < inner.bytes.len() {
            out.push(inner.varint()? as u32);
        }
        Ok(())
    }

    fn skip(&mut self, wire: u8) -> Result<(), DecodeError> {
        match wire {
            0 => self.varint().map(drop),
            1 => self.take(8).map(drop),
            2 => self.bytes().map(drop),
            5 => self.take(4).map(drop),
            other => Err(DecodeError::WireType(other)),
        }
    }
}

/// Encoding, for tests (this crate's and its users'): a writer of the same wire format, so a tile can be made field by
/// field and read back. Not a full encoder: one value a field, the specification's version 2 and extent 4096.
pub mod encode {
    pub fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    pub fn key(out: &mut Vec<u8>, field: u32, wire: u8) {
        varint(out, (u64::from(field) << 3) | u64::from(wire));
    }

    pub fn bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
        key(out, field, 2);
        varint(out, b.len() as u64);
        out.extend_from_slice(b);
    }

    pub fn packed(out: &mut Vec<u8>, field: u32, values: &[u32]) {
        let mut inner = Vec::new();
        for &v in values {
            varint(&mut inner, u64::from(v));
        }
        bytes(out, field, &inner);
    }

    pub fn zigzag(v: i32) -> u32 {
        ((v << 1) ^ (v >> 31)) as u32
    }

    /// A command stream: `MoveTo` to each path's first point, `LineTo` the rest, `ClosePath` when `close`.
    pub fn commands(paths: &[(&[[i32; 2]], bool)]) -> Vec<u32> {
        let mut out = Vec::new();
        let (mut x, mut y) = (0, 0);
        for (path, close) in paths {
            let mut emit = |out: &mut Vec<u32>, p: [i32; 2]| {
                out.push(zigzag(p[0] - x));
                out.push(zigzag(p[1] - y));
                x = p[0];
                y = p[1];
            };
            out.push(1 | (1 << 3));
            emit(&mut out, path[0]);
            if path.len() > 1 {
                out.push(2 | (((path.len() - 1) as u32) << 3));
                for &p in &path[1..] {
                    emit(&mut out, p);
                }
            }
            if *close {
                out.push(7 | (1 << 3));
            }
        }
        out
    }

    /// A feature: id, tags as (key, value) indices, its type and commands.
    pub fn feature(id: u64, tags: &[(u32, u32)], kind: u64, commands: &[u32]) -> Vec<u8> {
        let mut f = Vec::new();
        key(&mut f, 1, 0);
        varint(&mut f, id);
        let flat: Vec<u32> = tags.iter().flat_map(|&(k, v)| [k, v]).collect();
        packed(&mut f, 2, &flat);
        key(&mut f, 3, 0);
        varint(&mut f, kind);
        packed(&mut f, 4, commands);
        f
    }

    pub fn string_value(s: &str) -> Vec<u8> {
        let mut v = Vec::new();
        bytes(&mut v, 1, s.as_bytes());
        v
    }

    pub fn double_value(d: f64) -> Vec<u8> {
        let mut v = Vec::new();
        key(&mut v, 3, 1);
        v.extend_from_slice(&d.to_le_bytes());
        v
    }

    pub fn sint_value(i: i64) -> Vec<u8> {
        let mut v = Vec::new();
        key(&mut v, 6, 0);
        varint(&mut v, ((i << 1) ^ (i >> 63)) as u64);
        v
    }

    /// A layer of `features` with its key and value tables.
    pub fn layer(name: &str, keys: &[&str], values: &[Vec<u8>], features: &[Vec<u8>]) -> Vec<u8> {
        let mut l = Vec::new();
        key(&mut l, 15, 0);
        varint(&mut l, 2);
        bytes(&mut l, 1, name.as_bytes());
        for f in features {
            bytes(&mut l, 2, f);
        }
        for k in keys {
            bytes(&mut l, 3, k.as_bytes());
        }
        for v in values {
            bytes(&mut l, 4, v);
        }
        key(&mut l, 5, 0);
        varint(&mut l, 4096);
        l
    }

    pub fn tile(layers: &[Vec<u8>]) -> Vec<u8> {
        let mut t = Vec::new();
        for l in layers {
            bytes(&mut t, 3, l);
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::encode::*;
    use super::{DecodeError, GeomType, Geometry, decode, geometry, paths, ring_area2};

    #[test]
    fn the_specifications_examples_decode() {
        // MVT 2.1, 4.3.5: a point at (25, 17); a line through (2, 2), (2, 10), (10, 10); a polygon (3, 6), (8, 12),
        // (20, 34) closed.
        let (p, _) = paths(&[9, 50, 34]);
        assert_eq!(p, vec![vec![[25, 17]]]);
        let (p, _) = paths(&[9, 4, 4, 18, 0, 16, 16, 0]);
        assert_eq!(p, vec![vec![[2, 2], [2, 10], [10, 10]]]);
        let (p, closed) = paths(&[9, 6, 12, 18, 10, 12, 24, 44, 15]);
        assert_eq!(p, vec![vec![[3, 6], [8, 12], [20, 34]]]);
        assert_eq!(closed, vec![true]);
        // The multipolygon example: two exteriors, the second with a hole.
        let g = geometry(
            GeomType::Polygon,
            &[
                9, 0, 0, 26, 20, 0, 0, 20, 19, 0, 15, 9, 22, 2, 26, 18, 0, 0, 18, 17, 0, 15, 9, 4,
                13, 26, 0, 8, 8, 0, 0, 7, 15,
            ],
        );
        let Geometry::Polygons(polys) = g else {
            panic!("{g:?}")
        };
        assert_eq!(polys.len(), 2);
        assert_eq!(polys[0].exterior, vec![[0, 0], [10, 0], [10, 10], [0, 10]]);
        assert!(polys[0].holes.is_empty());
        assert_eq!(
            polys[1].exterior,
            vec![[11, 11], [20, 11], [20, 20], [11, 20]]
        );
        assert_eq!(
            polys[1].holes,
            vec![vec![[13, 13], [13, 17], [17, 17], [17, 13]]]
        );
    }

    #[test]
    fn a_tile_made_field_by_field_reads_back() {
        let road = feature(
            7,
            &[(0, 0), (1, 1)],
            2,
            &commands(&[(&[[0, 0], [100, 50], [4200, -30]], false)]),
        );
        let lake = feature(
            9,
            &[(0, 2), (2, 3)],
            3,
            &commands(&[
                (&[[0, 0], [400, 0], [400, 400], [0, 400]], true),
                (&[[100, 100], [100, 200], [200, 200], [200, 100]], true),
            ]),
        );
        let t = tile(&[
            layer(
                "transportation",
                &["class", "layer", "id"],
                &[
                    string_value("primary"),
                    sint_value(-1),
                    string_value("lake"),
                    double_value(12.5),
                ],
                &[road, lake],
            ),
            layer("empty", &[], &[], &[]),
        ]);
        let decoded = decode(&t).unwrap();
        assert_eq!(decoded.layers.len(), 2);
        let l = decoded.layer("transportation").unwrap();
        assert_eq!((l.version, l.extent), (2, 4096));
        assert_eq!(l.features.len(), 2);
        let f = &l.features[0];
        assert_eq!(f.id, Some(7));
        assert_eq!(l.str(f, "class"), Some("primary"));
        assert_eq!(l.number(f, "layer"), Some(-1.0));
        assert_eq!(
            f.geometry,
            Geometry::Lines(vec![vec![[0, 0], [100, 50], [4200, -30]]])
        );
        let g = &l.features[1];
        assert_eq!(l.number(g, "id"), Some(12.5));
        assert_eq!(l.tags(g).count(), 2);
        let Geometry::Polygons(p) = &g.geometry else {
            panic!()
        };
        // The exterior is clockwise as drawn (y south): positive; the hole the other way.
        assert!(ring_area2(&p[0].exterior) > 0);
        assert_eq!(p[0].holes.len(), 1);
        assert!(decoded.layer("empty").unwrap().features.is_empty());

        // Gzipped, as some servers send it.
        let gz = {
            let mut g = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
            g.extend(miniz_oxide::deflate::compress_to_vec(&t, 6));
            g.extend([0; 8]);
            g
        };
        assert_eq!(decode(&gz).unwrap(), decoded);
        // Only the layers asked for: the rest are read past, not kept.
        let only = super::decode_layers(&t, Some(&["empty"])).unwrap();
        assert_eq!(only.layers.len(), 1);
        assert_eq!(only.layers[0].name, "empty");
        assert_eq!(
            super::decode_layers(&gz, Some(&["transportation"]))
                .unwrap()
                .layers[0],
            *l
        );
    }

    #[test]
    fn bad_bytes_are_refused_not_panicked_on() {
        let t = tile(&[layer("x", &["a"], &[string_value("b")], &[])]);
        for cut in 1..t.len() {
            // A tile cut short anywhere either reads what was whole or says it was truncated.
            let _ = decode(&t[..cut]);
        }
        assert_eq!(decode(&[0x1a, 0x05, 0x0a]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x1b]), Err(DecodeError::WireType(3)));
        assert_eq!(decode(&[0x08]), Err(DecodeError::Truncated));
        // A malformed command stream stops where it ran out.
        assert_eq!(paths(&[9, 2, 2, 9]).0, vec![vec![[1, 1]]]);
        assert!(paths(&[18, 2]).0.is_empty());
        // A ring across the whole of i32 neither overflows nor loses its winding.
        assert!(
            ring_area2(&[
                [-i32::MAX, 0],
                [i32::MAX, 0],
                [i32::MAX, i32::MAX],
                [-i32::MAX, i32::MAX]
            ]) > 0
        );
        // A gzipped body that would inflate past the limit is refused.
        let bomb = {
            let mut g = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
            g.extend(miniz_oxide::deflate::compress_to_vec(
                &vec![0u8; super::MAX_INFLATED_BYTES + 1],
                6,
            ));
            g.extend([0; 8]);
            g
        };
        assert!(matches!(decode(&bomb), Err(DecodeError::Gzip(_))));
    }
}
