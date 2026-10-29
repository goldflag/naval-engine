//! Well-known binary, the GeoParquet `geometry` column's encoding: the points, line strings and multi line strings
//! the infrastructure's power uses (x the longitude, y the latitude). Polygons and the rest are not read.

/// A geometry as read: a point, or lines (a line string is one).
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Point([f64; 2]),
    Lines(Vec<Vec<[f64; 2]>>),
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.bytes.get(self.at..self.at + N)?.try_into().ok()?;
        self.at += N;
        Some(b)
    }

    fn u32(&mut self, little: bool) -> Option<u32> {
        let b = self.take::<4>()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    fn f64(&mut self, little: bool) -> Option<f64> {
        let b = self.take::<8>()?;
        Some(if little {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }

    /// A geometry's header: its byte order and type.
    fn header(&mut self) -> Option<(bool, u32)> {
        let [order] = self.take::<1>()?;
        let little = order == 1;
        Some((little, self.u32(little)?))
    }

    fn points(&mut self, little: bool) -> Option<Vec<[f64; 2]>> {
        let n = self.u32(little)? as usize;
        if n > (self.bytes.len() - self.at) / 16 {
            return None;
        }
        (0..n)
            .map(|_| Some([self.f64(little)?, self.f64(little)?]))
            .collect()
    }
}

/// The shape of `bytes`, if a point, a line string or a multi line string (2D).
pub fn decode(bytes: &[u8]) -> Option<Shape> {
    let mut c = Cursor { bytes, at: 0 };
    let (little, kind) = c.header()?;
    match kind {
        1 => Some(Shape::Point([c.f64(little)?, c.f64(little)?])),
        2 => Some(Shape::Lines(vec![c.points(little)?])),
        5 => {
            let n = c.u32(little)? as usize;
            let mut lines = Vec::with_capacity(n.min(1024));
            for _ in 0..n {
                let (little, kind) = c.header()?;
                if kind != 2 {
                    return None;
                }
                lines.push(c.points(little)?);
            }
            Some(Shape::Lines(lines))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn line_string(points: &[[f64; 2]]) -> Vec<u8> {
    let mut b = vec![1u8];
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&(points.len() as u32).to_le_bytes());
    for p in points {
        b.extend_from_slice(&p[0].to_le_bytes());
        b.extend_from_slice(&p[1].to_le_bytes());
    }
    b
}

#[cfg(test)]
pub(crate) fn point(p: [f64; 2]) -> Vec<u8> {
    let mut b = vec![1u8];
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&p[0].to_le_bytes());
    b.extend_from_slice(&p[1].to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_lines_and_multi_lines_decode() {
        assert_eq!(
            decode(&point([-118.4, 33.9])),
            Some(Shape::Point([-118.4, 33.9]))
        );
        let line = [[-118.0, 34.0], [-118.1, 34.1], [-118.2, 34.05]];
        assert_eq!(
            decode(&line_string(&line)),
            Some(Shape::Lines(vec![line.to_vec()]))
        );
        let mut multi = vec![1u8];
        multi.extend_from_slice(&5u32.to_le_bytes());
        multi.extend_from_slice(&2u32.to_le_bytes());
        multi.extend(line_string(&line[..2]));
        multi.extend(line_string(&line[1..]));
        assert_eq!(
            decode(&multi),
            Some(Shape::Lines(vec![line[..2].to_vec(), line[1..].to_vec()]))
        );
        // A polygon, and bytes cut short.
        let mut polygon = vec![1u8];
        polygon.extend_from_slice(&3u32.to_le_bytes());
        assert_eq!(decode(&polygon), None);
        assert_eq!(decode(&line_string(&line)[..20]), None);
    }
}
