//! A square's water from the snapshot: the polygons its bake reads, cut to the square's **held box** (its final window
//! grown by its halo and two lattice steps, [`held_box`], longitude continuous about the square), each in the square's
//! own longitude frame (a polygon west of −180° is shifted a turn east for square 179° E). The client keeps a square's
//! pieces in its source cache, and when they are gone cuts them again from the snapshot ([`Splitter`]: one pass for any
//! number of squares).
//!
//! **The cut changes no sub-point's share of the sea.** Each ring is clipped to the box by Sutherland–Hodgman, which
//! keeps a ring's winding about every point strictly inside the box (only the parts outside are replaced, by runs along
//! the box's edges). The box's edges lie on lattice steps, and the coverage's sub-points never do (they stand at odd
//! multiples of 1/12,800°), so the clipped polygons give every sub-point the bake reads its share from the snapshot's
//! own.
//!
//! The file ([`Pieces::encode`], `NWP1`): the magic, the snapshot's SHA-256, the square, the box, the polygons (each its
//! rings of [lat, lon] f64), then the SHA-256 of all before it.
use std::sync::Arc;

use naval_land_format::lattice::PER_DEGREE;
use naval_land_format::{Digest, SquareKey};

use super::snapshot::WaterRecord;
use crate::BakeError;
use crate::finish::Halo;
use crate::finish::coverage::{GeoPolygon, Water};
use crate::finish::lattice::Region;

pub const MAGIC: &[u8; 4] = b"NWP1";
/// Lattice steps the box holds past the halo (the coverage's sub-points reach half a step past it).
pub const MARGIN_STEPS: usize = 2;

/// The box square `key`'s bake reads the water in, lattice steps: south, north, west, east (longitude continuous about
/// the square).
pub fn held_box(key: SquareKey) -> [i32; 4] {
    let halo = Halo::of(key);
    let r = Region::window(key).grow(halo.rows() + MARGIN_STEPS, halo.columns() + MARGIN_STEPS);
    [r.i(r.rows - 1), r.top, r.left, r.j(r.columns - 1)]
}

fn degrees(b: [i32; 4]) -> [f64; 4] {
    b.map(|v| f64::from(v) / f64::from(PER_DEGREE))
}

/// A ring clipped to the box [south, north, west, east] by Sutherland–Hodgman (none when nothing of it is left).
fn clip_ring(ring: &[[f64; 2]], b: [f64; 4]) -> Option<Vec<[f64; 2]>> {
    // Each half-plane: the coordinate (0 latitude, 1 longitude), the bound, and whether a point must lie above it.
    let planes = [
        (0, b[0], true),
        (0, b[1], false),
        (1, b[2], true),
        (1, b[3], false),
    ];
    let mut points = ring.to_vec();
    for (axis, bound, above) in planes {
        if points.is_empty() {
            return None;
        }
        let keep = |p: &[f64; 2]| {
            if above {
                p[axis] >= bound
            } else {
                p[axis] <= bound
            }
        };
        let cross = |p: [f64; 2], q: [f64; 2]| {
            let t = (bound - p[axis]) / (q[axis] - p[axis]);
            let other = 1 - axis;
            let mut out = [0.0; 2];
            out[axis] = bound;
            out[other] = p[other] + t * (q[other] - p[other]);
            out
        };
        let n = points.len();
        let mut out = Vec::with_capacity(n + 4);
        for k in 0..n {
            let (p, q) = (points[k], points[(k + 1) % n]);
            match (keep(&p), keep(&q)) {
                (true, true) => out.push(q),
                (true, false) => out.push(cross(p, q)),
                (false, true) => {
                    out.push(cross(p, q));
                    out.push(q);
                }
                (false, false) => {}
            }
        }
        points = out;
    }
    (points.len() >= 3).then_some(points)
}

/// The polygon `record` cut to box `b` (degrees, longitude continuous), once for each whole turn in which it meets the
/// box, shifted into the box's frame.
pub fn clip(record: &WaterRecord, b: [f64; 4]) -> Vec<GeoPolygon> {
    let [south, north, west, east] = record.bounds;
    if north < b[0] || south > b[1] {
        return vec![];
    }
    let lo = ((b[2] - east) / 360.0).ceil() as i64;
    let hi = ((b[3] - west) / 360.0).floor() as i64;
    let mut out = vec![];
    for turn in lo..=hi {
        let shift = 360.0 * turn as f64;
        let rings: Vec<Vec<[f64; 2]>> = record
            .rings
            .iter()
            .filter_map(|ring| {
                let moved: Vec<[f64; 2]> = ring.iter().map(|p| [p[0], p[1] + shift]).collect();
                clip_ring(&moved, b)
            })
            .collect();
        if !rings.is_empty() {
            out.push(GeoPolygon::new(rings));
        }
    }
    out
}

/// A square's water.
#[derive(Clone, Debug, PartialEq)]
pub struct Pieces {
    pub key: SquareKey,
    /// The snapshot they were cut from.
    pub snapshot: Digest,
    pub held: [i32; 4],
    pub polygons: Vec<Arc<GeoPolygon>>,
}

impl Pieces {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.snapshot.0);
        for v in [self.key.lat, self.key.lon] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in self.held {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(self.polygons.len() as u32).to_le_bytes());
        for p in &self.polygons {
            out.extend_from_slice(&(p.rings.len() as u32).to_le_bytes());
            for ring in &p.rings {
                out.extend_from_slice(&(ring.len() as u32).to_le_bytes());
                for point in ring {
                    out.extend_from_slice(&point[0].to_le_bytes());
                    out.extend_from_slice(&point[1].to_le_bytes());
                }
            }
        }
        let digest = Digest::of(&out);
        out.extend_from_slice(&digest.0);
        out
    }

    /// Square `key`'s pieces from their file, refused unless whole, cut from `snapshot`, and `key`'s.
    pub fn decode(bytes: &[u8], key: SquareKey, snapshot: Digest) -> Result<Self, BakeError> {
        let bad = |why: &str| BakeError::Corrupt(format!("{key}'s water pieces: {why}"));
        if bytes.len() < 4 + 32 + 24 + 4 + 32 || &bytes[..4] != MAGIC {
            return Err(bad("not NWP1"));
        }
        let (body, sum) = bytes.split_at(bytes.len() - 32);
        if Digest::of(body).0 != sum {
            return Err(bad("their digest does not match"));
        }
        let mut r = Reader { body, at: 4 };
        let from = Digest(r.take(32).ok_or_else(|| bad("short"))?.try_into().unwrap());
        let mut ints = [0i32; 6];
        for v in &mut ints {
            *v = r.i32().ok_or_else(|| bad("short"))?;
        }
        let [lat, lon, held @ ..] = ints;
        if from != snapshot || (SquareKey { lat, lon }) != key || held != held_box(key) {
            return Err(bad("another snapshot's, square's or box's"));
        }
        let polygons = r.count().ok_or_else(|| bad("short"))?;
        let mut out = Vec::with_capacity(polygons.min(1 << 16));
        for _ in 0..polygons {
            let rings = r.count().ok_or_else(|| bad("short"))?;
            let mut rs = Vec::with_capacity(rings.min(1 << 16));
            for _ in 0..rings {
                let n = r.count().ok_or_else(|| bad("short"))?;
                let raw = n
                    .checked_mul(16)
                    .and_then(|len| r.take(len))
                    .ok_or_else(|| bad("short"))?;
                rs.push(
                    raw.as_chunks::<16>()
                        .0
                        .iter()
                        .map(|c| {
                            [
                                f64::from_le_bytes(c[..8].try_into().unwrap()),
                                f64::from_le_bytes(c[8..].try_into().unwrap()),
                            ]
                        })
                        .collect(),
                );
            }
            out.push(Arc::new(GeoPolygon::new(rs)));
        }
        let at = r.at;
        if at != body.len() {
            return Err(bad("bytes past its polygons"));
        }
        Ok(Self {
            key,
            snapshot,
            held,
            polygons: out,
        })
    }
}

/// A file read front to back.
struct Reader<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.body.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn count(&mut self) -> Option<usize> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize)
    }
}

impl Water for Pieces {
    /// Every piece, for any range inside the held box; a range past it is refused (the bake asked for water this square
    /// was not cut for).
    fn ocean(
        &self,
        south: f64,
        north: f64,
        west: f64,
        east: f64,
    ) -> Result<Vec<Arc<GeoPolygon>>, BakeError> {
        let b = degrees(self.held);
        if south < b[0] || north > b[1] || west < b[2] || east > b[3] {
            return Err(BakeError::Missing(format!(
                "water for {south}..{north}, {west}..{east}: past {}'s held box",
                self.key
            )));
        }
        Ok(self.polygons.clone())
    }
}

/// Cuts the pieces of some squares in one pass over the snapshot.
pub struct Splitter {
    want: Vec<(SquareKey, [f64; 4], Vec<Arc<GeoPolygon>>)>,
}

impl Splitter {
    pub fn new(keys: impl IntoIterator<Item = SquareKey>) -> Self {
        Self {
            want: keys
                .into_iter()
                .map(|k| (k, degrees(held_box(k)), vec![]))
                .collect(),
        }
    }

    pub fn add(&mut self, record: &WaterRecord) {
        for (_, b, got) in &mut self.want {
            got.extend(clip(record, *b).into_iter().map(Arc::new));
        }
    }

    pub fn finish(self, snapshot: Digest) -> Vec<Pieces> {
        self.want
            .into_iter()
            .map(|(key, _, polygons)| Pieces {
                key,
                snapshot,
                held: held_box(key),
                polygons,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finish::coverage::ocean_cover;

    struct Whole(Vec<Arc<GeoPolygon>>);
    impl Water for Whole {
        fn ocean(&self, _: f64, _: f64, _: f64, _: f64) -> Result<Vec<Arc<GeoPolygon>>, BakeError> {
            Ok(self.0.clone())
        }
    }

    fn record(index: usize, rings: Vec<Vec<[f64; 2]>>) -> WaterRecord {
        let p = GeoPolygon::new(rings.clone());
        WaterRecord {
            index,
            rings,
            bounds: p.bounds,
        }
    }

    /// A wiggly ring about (lat, lon), `r` degrees across.
    fn wiggle(lat: f64, lon: f64, r: f64, n: usize, k: f64) -> Vec<[f64; 2]> {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                let rr = r * (1.0 + 0.3 * (k * a).sin());
                [lat + rr * a.sin(), lon + rr * a.cos()]
            })
            .collect()
    }

    #[test]
    fn cut_pieces_cover_every_sub_point_as_the_whole_polygons_do() {
        // Square 179° E and 180° W: water crossing their box's edges and the date line, a hole (an island) cut by the
        // box, and a polygon from the far side of ±180°.
        for key in [
            SquareKey { lat: -17, lon: 179 },
            SquareKey {
                lat: -17,
                lon: -180,
            },
        ] {
            let records = vec![
                record(
                    0,
                    vec![
                        wiggle(-16.5, 179.6, 0.7, 400, 7.0),
                        wiggle(-16.2, 179.9, 0.25, 90, 3.0),
                    ],
                ),
                record(1, vec![wiggle(-17.0, -179.7, 0.5, 300, 5.0)]),
                record(2, vec![wiggle(-15.9, 178.95, 0.2, 60, 4.0)]),
            ];
            let mut split = Splitter::new([key]);
            for r in &records {
                split.add(r);
            }
            let pieces = split.finish(Digest::of(b"snapshot")).remove(0);
            assert!(!pieces.polygons.is_empty());
            let back = Pieces::decode(&pieces.encode(), key, Digest::of(b"snapshot")).unwrap();
            assert_eq!(back, pieces);
            assert!(Pieces::decode(&pieces.encode(), key, Digest::of(b"other")).is_err());
            let whole = Whole(
                records
                    .iter()
                    .map(|r| Arc::new(GeoPolygon::new(r.rings.clone())))
                    .collect(),
            );
            // The window and the halo as the bake reads them, in rows of 64 to keep the test quick.
            let halo = Halo::of(key);
            let held = Region::window(key).grow(halo.rows(), halo.columns());
            for band in 0..held.rows / 64 {
                let region = Region {
                    top: held.top - (band * 64) as i32,
                    left: held.left,
                    rows: 8,
                    columns: held.columns,
                };
                let a = ocean_cover(&region, &pieces).unwrap();
                let b = ocean_cover(&region, &whole).unwrap();
                assert_eq!(a, b, "{key}, rows from {}", region.top);
            }
        }
    }

    #[test]
    fn a_range_past_the_box_is_refused() {
        let key = SquareKey { lat: 40, lon: 10 };
        let pieces = Splitter::new([key]).finish(Digest::of(b"s")).remove(0);
        let b = degrees(held_box(key));
        assert!(pieces.ocean(b[0], b[1], b[2], b[3]).is_ok());
        assert!(pieces.ocean(b[0] - 0.01, b[1], b[2], b[3]).is_err());
    }
}
