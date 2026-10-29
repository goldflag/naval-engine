//! Footprint geometry on the ground plane: rings of (x, z) metres (x east, z south, as the chart), their area and
//! orientation, simplification, the minimum-area rectangle about them and ear-clipping triangulation for flat roofs.
//!
//! A ring is a closed polygon without its closing point repeated. "Counter-clockwise" here is the (x, z) plane's own
//! sense: positive [`signed_area`]. Seen from above (+y toward the eye, x right, z down the screen) such a ring turns
//! clockwise; its outward normal from edge `a → b` is `(b.z - a.z, -(b.x - a.x))`.

/// Twice the signed area of `ring`, (x, z) sense: positive counter-clockwise.
pub fn signed_area2(ring: &[[f32; 2]]) -> f32 {
    let n = ring.len();
    let mut sum = 0.0f64;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        sum += f64::from(a[0]) * f64::from(b[1]) - f64::from(b[0]) * f64::from(a[1]);
    }
    sum as f32
}

/// The signed area of `ring` (m²), positive counter-clockwise.
pub fn signed_area(ring: &[[f32; 2]]) -> f32 {
    0.5 * signed_area2(ring)
}

/// The area of `ring` (m²).
pub fn area(ring: &[[f32; 2]]) -> f32 {
    signed_area(ring).abs()
}

/// The ring's area centroid (its vertex mean for a degenerate ring).
pub fn centroid(ring: &[[f32; 2]]) -> [f32; 2] {
    let n = ring.len();
    if n == 0 {
        return [0.0, 0.0];
    }
    let mut a2 = 0.0f64;
    let (mut cx, mut cz) = (0.0f64, 0.0f64);
    for i in 0..n {
        let p = ring[i];
        let q = ring[(i + 1) % n];
        let (px, pz, qx, qz) = (
            f64::from(p[0]),
            f64::from(p[1]),
            f64::from(q[0]),
            f64::from(q[1]),
        );
        let cross = px * qz - qx * pz;
        a2 += cross;
        cx += (px + qx) * cross;
        cz += (pz + qz) * cross;
    }
    if a2.abs() < 1e-9 {
        let s = ring
            .iter()
            .fold([0.0f32; 2], |s, p| [s[0] + p[0], s[1] + p[1]]);
        return [s[0] / n as f32, s[1] / n as f32];
    }
    [(cx / (3.0 * a2)) as f32, (cz / (3.0 * a2)) as f32]
}

/// The ring's perimeter (m).
pub fn perimeter(ring: &[[f32; 2]]) -> f32 {
    let n = ring.len();
    (0..n).map(|i| distance(ring[i], ring[(i + 1) % n])).sum()
}

pub fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Distance from `p` to the segment `a`-`b`.
pub fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    if len2 < 1e-12 {
        return distance(p, a);
    }
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0);
    distance(p, [a[0] + t * d[0], a[1] + t * d[1]])
}

/// `ring` cleaned for extrusion: the closing point and repeated points dropped, counter-clockwise, and every vertex
/// whose removal moves the outline less than `tolerance` (m) removed, the least significant first, down to three. None
/// when what is left has no area.
pub fn simplify(ring: &[[f32; 2]], tolerance: f32) -> Option<Vec<[f32; 2]>> {
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(ring.len());
    for &p in ring {
        if !(p[0].is_finite() && p[1].is_finite()) {
            return None;
        }
        if out.last().is_none_or(|q| distance(*q, p) > 0.05) {
            out.push(p);
        }
    }
    while out.len() > 1 && distance(out[0], out[out.len() - 1]) <= 0.05 {
        out.pop();
    }
    if out.len() < 3 {
        return None;
    }
    if signed_area2(&out) < 0.0 {
        out.reverse();
    }
    loop {
        let n = out.len();
        if n <= 3 {
            break;
        }
        let mut best = (f32::INFINITY, 0usize);
        for i in 0..n {
            let d = segment_distance(out[i], out[(i + n - 1) % n], out[(i + 1) % n]);
            if d < best.0 {
                best = (d, i);
            }
        }
        if best.0 >= tolerance {
            break;
        }
        out.remove(best.1);
    }
    (signed_area2(&out) > 1e-3).then_some(out)
}

/// The convex hull of `points` (counter-clockwise, Andrew's monotone chain).
pub fn convex_hull(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut p: Vec<[f32; 2]> = points.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut hull: Vec<[f32; 2]> = Vec::with_capacity(p.len() * 2);
    for &q in &p {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
            hull.pop();
        }
        hull.push(q);
    }
    let lower = hull.len() + 1;
    for &q in p.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
            hull.pop();
        }
        hull.push(q);
    }
    hull.pop();
    hull
}

/// The smallest rectangle about a ring: its centre, unit axis along its longer side, and half lengths (long, short).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub centre: [f32; 2],
    pub axis: [f32; 2],
    pub half: [f32; 2],
}

impl Rect {
    pub fn area(&self) -> f32 {
        4.0 * self.half[0] * self.half[1]
    }

    /// Its corners, counter-clockwise ((x, z) sense), starting at the long side's -axis, -normal end.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let a = self.axis;
        // The normal to the axis such that (axis, normal) is counter-clockwise in (x, z).
        let n = [-a[1], a[0]];
        let [l, s] = self.half;
        let c = self.centre;
        let at = |u: f32, v: f32| [c[0] + a[0] * u + n[0] * v, c[1] + a[1] * u + n[1] * v];
        [at(-l, -s), at(l, -s), at(l, s), at(-l, s)]
    }
}

/// The minimum-area rectangle about `ring` (rotating calipers over its hull's edges).
pub fn min_area_rect(ring: &[[f32; 2]]) -> Option<Rect> {
    let hull = convex_hull(ring);
    if hull.len() < 3 {
        return None;
    }
    let mut best: Option<(f32, Rect)> = None;
    let n = hull.len();
    for i in 0..n {
        let a = hull[i];
        let b = hull[(i + 1) % n];
        let len = distance(a, b);
        if len < 1e-4 {
            continue;
        }
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let v = [-u[1], u[0]];
        let (mut lo_u, mut hi_u, mut lo_v, mut hi_v) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in &hull {
            let d = [p[0] - a[0], p[1] - a[1]];
            let pu = d[0] * u[0] + d[1] * u[1];
            let pv = d[0] * v[0] + d[1] * v[1];
            lo_u = lo_u.min(pu);
            hi_u = hi_u.max(pu);
            lo_v = lo_v.min(pv);
            hi_v = hi_v.max(pv);
        }
        let area = (hi_u - lo_u) * (hi_v - lo_v);
        if best.as_ref().is_none_or(|(b, _)| area < *b) {
            let (cu, cv) = (0.5 * (lo_u + hi_u), 0.5 * (lo_v + hi_v));
            let centre = [a[0] + u[0] * cu + v[0] * cv, a[1] + u[1] * cu + v[1] * cv];
            let (hu, hv) = (0.5 * (hi_u - lo_u), 0.5 * (hi_v - lo_v));
            let rect = if hu >= hv {
                Rect {
                    centre,
                    axis: u,
                    half: [hu, hv],
                }
            } else {
                Rect {
                    centre,
                    axis: v,
                    half: [hv, hu],
                }
            };
            best = Some((area, rect));
        }
    }
    best.map(|(_, r)| r)
}

/// Whether point `p` lies inside `ring` (even-odd).
pub fn contains(ring: &[[f32; 2]], p: [f32; 2]) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// How round a ring is: 4π · area / perimeter², 1 for a circle, 0.785 for a square, about 0.95 for an octagon.
pub fn roundness(ring: &[[f32; 2]]) -> f32 {
    let p = perimeter(ring);
    if p <= 1e-6 {
        return 0.0;
    }
    4.0 * std::f32::consts::PI * area(ring) / (p * p)
}

/// A regular polygon of `sides` about `centre` with circumradius `radius`, counter-clockwise ((x, z) sense), its first
/// corner at angle `phase` (radians from +x toward +z).
pub fn regular(centre: [f32; 2], radius: f32, sides: u32, phase: f32) -> Vec<[f32; 2]> {
    (0..sides)
        .map(|k| {
            let a = phase + std::f32::consts::TAU * k as f32 / sides as f32;
            [centre[0] + radius * a.cos(), centre[1] + radius * a.sin()]
        })
        .collect()
}

/// The strip `half` metres either side of a polyline, as one counter-clockwise ring (mitred joins, a miter held to
/// three times the half width): a pier or a jetty drawn from its line. None for a line with no length.
pub fn buffer_line(line: &[[f32; 2]], half: f32) -> Option<Vec<[f32; 2]>> {
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(line.len());
    for &p in line {
        if pts.last().is_none_or(|q| distance(*q, p) > 0.05) {
            pts.push(p);
        }
    }
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len();
    let normal = |a: [f32; 2], b: [f32; 2]| {
        let l = distance(a, b).max(1e-6);
        [-(b[1] - a[1]) / l, (b[0] - a[0]) / l]
    };
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let m = if i == 0 {
            normal(pts[0], pts[1])
        } else if i == n - 1 {
            normal(pts[n - 2], pts[n - 1])
        } else {
            let (a, b) = (normal(pts[i - 1], pts[i]), normal(pts[i], pts[i + 1]));
            let s = [a[0] + b[0], a[1] + b[1]];
            let l = s[0].hypot(s[1]);
            if l < 1e-3 {
                a
            } else {
                // The miter: the mean normal lengthened to keep the strip's width, held to 3 half widths.
                let m = [s[0] / l, s[1] / l];
                let cos = (m[0] * a[0] + m[1] * a[1]).max(1.0 / 3.0);
                [m[0] / cos, m[1] / cos]
            }
        };
        let p = pts[i];
        left.push([p[0] + m[0] * half, p[1] + m[1] * half]);
        right.push([p[0] - m[0] * half, p[1] - m[1] * half]);
    }
    let mut ring = right;
    ring.extend(left.into_iter().rev());
    if signed_area2(&ring) < 0.0 {
        ring.reverse();
    }
    (signed_area2(&ring) > 1e-3).then_some(ring)
}

/// Whether `p` lies inside a polygon's exterior and outside its holes.
pub fn contains_with_holes(exterior: &[[f32; 2]], holes: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
    contains(exterior, p) && !holes.iter().any(|h| contains(h, p))
}

/// The point a ring holds farthest from its edges (to a ninth of its box, among a grid's points and its centroid), and
/// that distance: where a concave footprint has the most room. None for a ring that holds none of them.
pub fn inner_point(ring: &[[f32; 2]]) -> Option<([f32; 2], f32)> {
    if ring.len() < 3 {
        return None;
    }
    let (lo, hi) = ring.iter().fold(
        ([f32::MAX, f32::MAX], [f32::MIN, f32::MIN]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    );
    let steps = 9;
    let grid = (0..=steps).flat_map(|j| {
        (0..=steps).map(move |i| {
            [
                lo[0] + (hi[0] - lo[0]) * i as f32 / steps as f32,
                lo[1] + (hi[1] - lo[1]) * j as f32 / steps as f32,
            ]
        })
    });
    std::iter::once(centroid(ring))
        .chain(grid)
        .filter(|p| contains(ring, *p))
        .map(|p| (p, ring_distance(ring, p)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// The least distance from `p` to a ring's edges.
pub fn ring_distance(ring: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let n = ring.len();
    (0..n)
        .map(|i| segment_distance(p, ring[i], ring[(i + 1) % n]))
        .fold(f32::INFINITY, f32::min)
}

/// Which side of the edge `a`-`b` of a counter-clockwise convex polygon `p` lies on: positive inside.
fn side(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// `ring` cut to the convex, counter-clockwise polygon `clip` (Sutherland and Hodgman): a map tile's polygon held to
/// the tile proper, so the tiles' pieces of a feature meet without overlapping. Empty when nothing is left.
pub fn clip_polygon(ring: &[[f32; 2]], clip: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = ring.to_vec();
    let n = clip.len();
    for i in 0..n {
        if out.is_empty() {
            break;
        }
        let (a, b) = (clip[i], clip[(i + 1) % n]);
        let input = std::mem::take(&mut out);
        let m = input.len();
        for k in 0..m {
            let (p, q) = (input[k], input[(k + 1) % m]);
            let (sp, sq) = (side(a, b, p), side(a, b, q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                out.push(lerp2(p, q, sp / (sp - sq)));
            }
        }
    }
    if out.len() < 3 { Vec::new() } else { out }
}

/// A polyline cut to the convex, counter-clockwise polygon `clip`: the pieces inside it (Cyrus and Beck per segment,
/// joined where they run on).
pub fn clip_polyline(line: &[[f32; 2]], clip: &[[f32; 2]]) -> Vec<Vec<[f32; 2]>> {
    let n = clip.len();
    let mut pieces: Vec<Vec<[f32; 2]>> = Vec::new();
    for w in line.windows(2) {
        let (p, q) = (w[0], w[1]);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for i in 0..n {
            let (a, b) = (clip[i], clip[(i + 1) % n]);
            let (sp, sq) = (side(a, b, p), side(a, b, q));
            if sp < 0.0 && sq < 0.0 {
                t0 = 1.0;
                t1 = 0.0;
                break;
            }
            if sp < 0.0 {
                t0 = t0.max(sp / (sp - sq));
            } else if sq < 0.0 {
                t1 = t1.min(sp / (sp - sq));
            }
        }
        if t0 >= t1 {
            continue;
        }
        let (a, b) = (lerp2(p, q, t0), lerp2(p, q, t1));
        match pieces.last_mut() {
            Some(last) if last.last().is_some_and(|l| distance(*l, a) < 1e-4) => last.push(b),
            _ => pieces.push(vec![a, b]),
        }
    }
    pieces
}

/// Triangles covering a counter-clockwise `ring` (ear clipping), as index triples into it, each counter-clockwise in
/// (x, z). A ring that will not clip (self-intersecting) falls back to a fan about its first vertex.
pub fn triangulate(ring: &[[f32; 2]]) -> Vec<[u32; 3]> {
    let n = ring.len();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return out;
    }
    let mut left: Vec<usize> = (0..n).collect();
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut guard = 0usize;
    while left.len() > 3 && guard < n * n {
        guard += 1;
        let m = left.len();
        let mut clipped = false;
        for k in 0..m {
            let (ia, ib, ic) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            let (a, b, c) = (ring[ia], ring[ib], ring[ic]);
            if cross(a, b, c) <= 1e-6 {
                continue;
            }
            let blocked = left.iter().any(|&i| {
                if i == ia || i == ib || i == ic {
                    return false;
                }
                let p = ring[i];
                cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
            });
            if blocked {
                continue;
            }
            out.push([ia as u32, ib as u32, ic as u32]);
            left.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if left.len() == 3 {
        out.push([left[0] as u32, left[1] as u32, left[2] as u32]);
    } else if left.len() > 3 {
        for k in 1..left.len() - 1 {
            out.push([left[0] as u32, left[k] as u32, left[k + 1] as u32]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l_shape() -> Vec<[f32; 2]> {
        // 20 x 20 with a 10 x 10 corner out: 300 m².
        vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 10.0],
            [10.0, 10.0],
            [10.0, 20.0],
            [0.0, 20.0],
        ]
    }

    #[test]
    fn rings_are_cleaned_oriented_and_simplified() {
        let mut ring = l_shape();
        ring.reverse();
        ring.push(ring[0]);
        // A point nearly on the edge from (20, 0) to (0, 0), and a repeat.
        ring.insert(5, [10.0, 0.02]);
        ring.insert(5, [10.0, 0.02]);
        let s = simplify(&ring, 0.5).unwrap();
        assert_eq!(s.len(), 6, "{s:?}");
        assert!(signed_area(&s) > 0.0);
        assert!((area(&s) - 300.0).abs() < 0.01);
        assert!(simplify(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0.1).is_none());
        let c = centroid(&s);
        // The L's centroid: (5/12 of 20... ) by moments: x = (200*10 + 100*5)/300 = 8.33.
        assert!(
            (c[0] - 8.333).abs() < 0.01 && (c[1] - 8.333).abs() < 0.01,
            "{c:?}"
        );
    }

    #[test]
    fn the_rectangle_about_a_rotated_box_is_the_box() {
        let (cos, sin) = (0.5f32.cos(), 0.5f32.sin());
        let box_ = [[-6.0f32, -4.0], [6.0, -4.0], [6.0, 4.0], [-6.0, 4.0]]
            .map(|[x, z]| [100.0 + x * cos - z * sin, 50.0 + x * sin + z * cos]);
        let r = min_area_rect(&box_).unwrap();
        assert!(
            (r.half[0] - 6.0).abs() < 1e-3 && (r.half[1] - 4.0).abs() < 1e-3,
            "{r:?}"
        );
        assert!((r.centre[0] - 100.0).abs() < 1e-3 && (r.centre[1] - 50.0).abs() < 1e-3);
        assert!((r.axis[0].abs() - cos).abs() < 1e-3);
        let corners = r.corners();
        assert!(signed_area(&corners) > 0.0);
        assert!((area(&corners) - 96.0).abs() < 1e-2);
    }

    #[test]
    fn an_l_shape_is_triangulated_whole() {
        let ring = l_shape();
        let tris = triangulate(&ring);
        assert_eq!(tris.len(), 4);
        let covered: f32 = tris
            .iter()
            .map(|t| {
                signed_area(&[
                    ring[t[0] as usize],
                    ring[t[1] as usize],
                    ring[t[2] as usize],
                ])
            })
            .sum();
        assert!((covered - 300.0).abs() < 1e-3, "{covered}");
        for t in &tris {
            assert!(
                signed_area(&[
                    ring[t[0] as usize],
                    ring[t[1] as usize],
                    ring[t[2] as usize]
                ]) > 0.0
            );
        }
        assert!(contains(&ring, [5.0, 15.0]));
        assert!(!contains(&ring, [15.0, 15.0]));
    }

    #[test]
    fn round_rings_and_strips() {
        let circle = regular([10.0, -4.0], 8.0, 16, 0.3);
        assert!(signed_area(&circle) > 0.0);
        assert!(roundness(&circle) > 0.98);
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!((roundness(&square) - std::f32::consts::FRAC_PI_4).abs() < 1e-4);
        assert!(roundness(&regular([0.0, 0.0], 5.0, 8, 0.0)) > 0.94);
        // A bent pier 4 m wide: counter-clockwise, about its length times its width.
        let strip = buffer_line(&[[0.0, 0.0], [50.0, 0.0], [50.0, 30.0]], 2.0).unwrap();
        assert!(signed_area(&strip) > 0.0);
        assert!((area(&strip) - 80.0 * 4.0).abs() < 2.0, "{}", area(&strip));
        assert!(contains(&strip, [25.0, 1.5]) && !contains(&strip, [25.0, 2.5]));
        assert!(buffer_line(&[[1.0, 1.0], [1.0, 1.01]], 2.0).is_none());
        let holed = vec![vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]]];
        assert!(contains_with_holes(&square, &holed, [2.0, 2.0]));
        assert!(!contains_with_holes(&square, &holed, [5.0, 5.0]));
        assert!((ring_distance(&square, [5.0, 3.0]) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn features_are_cut_to_their_tiles() {
        // Two tiles side by side share an edge: a square across it is cut into two halves that meet without overlap.
        let west = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let east = [[10.0, 0.0], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0]];
        let across = [[6.0, 2.0], [14.0, 2.0], [14.0, 8.0], [6.0, 8.0]];
        let (a, b) = (clip_polygon(&across, &west), clip_polygon(&across, &east));
        assert!((area(&a) - 24.0).abs() < 1e-4 && (area(&b) - 24.0).abs() < 1e-4);
        assert!(clip_polygon(&[[30.0, 0.0], [31.0, 0.0], [31.0, 1.0]], &west).is_empty());
        // A line across both and back: its pieces in the west tile.
        let line = [[5.0, 5.0], [15.0, 5.0], [15.0, 6.0], [5.0, 6.0]];
        let pieces = clip_polyline(&line, &west);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert_eq!(pieces[0], vec![[5.0, 5.0], [10.0, 5.0]]);
        assert_eq!(pieces[1], vec![[10.0, 6.0], [5.0, 6.0]]);
        let inside = clip_polyline(&[[1.0, 1.0], [2.0, 2.0], [3.0, 1.0]], &west);
        assert_eq!(inside, vec![vec![[1.0, 1.0], [2.0, 2.0], [3.0, 1.0]]]);
    }
}
