//! The shapes buildings are made of beyond walls and roofs: prisms (a tank, a chimney, a lighthouse's tower, a
//! minaret), cones and pyramids (spires, a tank's roof, a lantern's cap), domes, boxes and beams (a crane's legs, jib and
//! boom), and the extras built from them ([`MeshBuilder::extra`]): a church's tower and spire, a dome on its drum, a
//! minaret, a stack, a lighthouse's tower, gallery and lantern, a quay's crane. Every face is wound counter-clockwise
//! seen from outside, its normals outward (smooth about a round shape, flat on a box).

use super::mesh::{
    MeshBuilder, PART_BANDED, PART_DOME, PART_GABLE, PART_GLASS, PART_ROOF, PART_STEEL, PART_WALL,
    style_of,
};
use super::{Building, Cap, Extra, Kind, Lod};

const TAU: f32 = std::f32::consts::TAU;

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How a prism's walls are laid out for the shader: `around` facade units about it (bays), and its height mapped to
/// facade rows by `rows_at` (storeys, or bands for a banded part).
pub struct Facade<'a> {
    pub around: f32,
    pub rows_at: &'a dyn Fn(f32) -> f32,
    pub rows: i8,
}

impl MeshBuilder {
    /// A face whose vertices carry their own normals (a round surface), fanned from its first point and wound to face
    /// the normals' mean.
    pub(crate) fn smooth_face(
        &mut self,
        points: &[[f32; 3]],
        normals: &[[f32; 3]],
        facade: &[[f32; 2]],
        rows: i8,
        style: [u8; 4],
    ) {
        if points.len() < 3 {
            return;
        }
        let mean = normals.iter().fold([0.0; 3], |a, n| add(a, *n));
        // The face's own normal (the largest of its fan's crossings, a degenerate first triangle aside).
        let mut geometric = [0.0f32; 3];
        for k in 1..points.len() - 1 {
            let c = cross(sub(points[k], points[0]), sub(points[k + 1], points[0]));
            geometric = add(geometric, c);
        }
        let flip = dot(geometric, mean) < 0.0;
        let first = self.vertex_count();
        for ((p, n), f) in points.iter().zip(normals).zip(facade) {
            self.push_vertex(*p, *n, *f, rows, style);
        }
        for k in 1..points.len() as u32 - 1 {
            if flip {
                self.push_triangle([first, first + k + 1, first + k]);
            } else {
                self.push_triangle([first, first + k, first + k + 1]);
            }
        }
    }

    /// A vertical prism of `sides` about `c` from `y0` to `y1`, radius `r0` at its foot and `r1` at its top (corners at
    /// `phase` radians), smooth (radial normals) or faceted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prism(
        &mut self,
        c: [f32; 2],
        r0: f32,
        r1: f32,
        y0: f32,
        y1: f32,
        sides: u32,
        phase: f32,
        smooth: bool,
        facade: &Facade,
        style: [u8; 4],
    ) {
        let height = (y1 - y0).max(1e-4);
        let normal_at = |a: f32| normalize([a.cos() * height, r0 - r1, a.sin() * height]);
        for k in 0..sides {
            let a0 = phase + TAU * k as f32 / sides as f32;
            let a1 = phase + TAU * (k + 1) as f32 / sides as f32;
            let at = |a: f32, r: f32, y: f32| [c[0] + r * a.cos(), y, c[1] + r * a.sin()];
            let (n0, n1) = if smooth {
                (normal_at(a0), normal_at(a1))
            } else {
                let n = normal_at(0.5 * (a0 + a1));
                (n, n)
            };
            let (u0, u1) = (
                facade.around * k as f32 / sides as f32,
                facade.around * (k + 1) as f32 / sides as f32,
            );
            let (v0, v1) = ((facade.rows_at)(y0), (facade.rows_at)(y1));
            self.smooth_face(
                &[
                    at(a0, r0, y0),
                    at(a1, r0, y0),
                    at(a1, r1, y1),
                    at(a0, r1, y1),
                ],
                &[n0, n1, n1, n0],
                &[[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
                facade.rows,
                style,
            );
        }
    }

    /// A flat regular polygon at `y` (a prism's top), facing up, or down when `down`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn disc(
        &mut self,
        c: [f32; 2],
        r: f32,
        y: f32,
        sides: u32,
        phase: f32,
        down: bool,
        style: [u8; 4],
    ) {
        let n = if down {
            [0.0, -1.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let points: Vec<[f32; 3]> = (0..sides)
            .map(|k| {
                let a = phase + TAU * k as f32 / sides as f32;
                [c[0] + r * a.cos(), y, c[1] + r * a.sin()]
            })
            .collect();
        let facade: Vec<[f32; 2]> = points.iter().map(|p| [p[0] - c[0], p[2] - c[1]]).collect();
        let normals = vec![n; points.len()];
        self.smooth_face(&points, &normals, &facade, 0, style);
    }

    /// A cone or pyramid of `sides` from radius `r` at `y0` to its apex at `y1`. A roof part's facade is metres down
    /// the slope from the apex and along it, for tile courses.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cone(
        &mut self,
        c: [f32; 2],
        r: f32,
        y0: f32,
        y1: f32,
        sides: u32,
        phase: f32,
        smooth: bool,
        style: [u8; 4],
    ) {
        let rise = y1 - y0;
        let slope = (r * r + rise * rise).sqrt();
        let normal_at = |a: f32| normalize([a.cos() * rise, r, a.sin() * rise]);
        let apex = [c[0], y1, c[1]];
        for k in 0..sides {
            let a0 = phase + TAU * k as f32 / sides as f32;
            let a1 = phase + TAU * (k + 1) as f32 / sides as f32;
            let at = |a: f32| [c[0] + r * a.cos(), y0, c[1] + r * a.sin()];
            let mid = normal_at(0.5 * (a0 + a1));
            let (n0, n1) = if smooth {
                (normal_at(a0), normal_at(a1))
            } else {
                (mid, mid)
            };
            let arc = r * TAU / sides as f32;
            self.smooth_face(
                &[at(a0), at(a1), apex],
                &[n0, n1, mid],
                &[
                    [slope, arc * k as f32],
                    [slope, arc * (k + 1) as f32],
                    [0.0, arc * (k as f32 + 0.5)],
                ],
                0,
                style,
            );
        }
    }

    /// A dome: `rings` bands of `sides` from radius `r` at `y0` up to its crown `height` above.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dome(
        &mut self,
        c: [f32; 2],
        r: f32,
        y0: f32,
        height: f32,
        sides: u32,
        rings: u32,
        style: [u8; 4],
    ) {
        let point = |a: f32, phi: f32| {
            [
                c[0] + r * phi.cos() * a.cos(),
                y0 + height * phi.sin(),
                c[1] + r * phi.cos() * a.sin(),
            ]
        };
        let normal = |a: f32, phi: f32| {
            normalize([
                phi.cos() * a.cos() / r.max(1e-3),
                phi.sin() / height.max(1e-3),
                phi.cos() * a.sin() / r.max(1e-3),
            ])
        };
        for i in 0..rings {
            let p0 = std::f32::consts::FRAC_PI_2 * i as f32 / rings as f32;
            let p1 = std::f32::consts::FRAC_PI_2 * (i + 1) as f32 / rings as f32;
            for k in 0..sides {
                let a0 = TAU * k as f32 / sides as f32;
                let a1 = TAU * (k + 1) as f32 / sides as f32;
                let f = |a: f32, p: f32| [a * r, (std::f32::consts::FRAC_PI_2 - p) * r];
                if i + 1 == rings {
                    self.smooth_face(
                        &[point(a0, p0), point(a1, p0), point(0.5 * (a0 + a1), p1)],
                        &[normal(a0, p0), normal(a1, p0), [0.0, 1.0, 0.0]],
                        &[f(a0, p0), f(a1, p0), f(0.5 * (a0 + a1), p1)],
                        0,
                        style,
                    );
                } else {
                    self.smooth_face(
                        &[point(a0, p0), point(a1, p0), point(a1, p1), point(a0, p1)],
                        &[
                            normal(a0, p0),
                            normal(a1, p0),
                            normal(a1, p1),
                            normal(a0, p1),
                        ],
                        &[f(a0, p0), f(a1, p0), f(a1, p1), f(a0, p1)],
                        0,
                        style,
                    );
                }
            }
        }
    }

    /// A beam of square section `w` from `a` to `b`: four sides and two ends, flat-shaded.
    pub(crate) fn beam(&mut self, a: [f32; 3], b: [f32; 3], w: f32, style: [u8; 4]) {
        let d = normalize(sub(b, a));
        let up = if d[1].abs() > 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u = scale(normalize(cross(d, up)), 0.5 * w);
        let v = scale(normalize(cross(u, d)), 0.5 * w);
        let corner = |p: [f32; 3], su: f32, sv: f32| add(add(p, scale(u, su)), scale(v, sv));
        let ring = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
        let length = dot(sub(b, a), d);
        for k in 0..4 {
            let (s0, t0) = ring[k];
            let (s1, t1) = ring[(k + 1) % 4];
            let out = add(scale(u, s0 + s1), scale(v, t0 + t1));
            self.face(
                &[
                    corner(a, s0, t0),
                    corner(a, s1, t1),
                    corner(b, s1, t1),
                    corner(b, s0, t0),
                ],
                &[[0.0, 0.0], [1.0, 0.0], [1.0, length], [0.0, length]],
                out,
                0,
                style,
            );
        }
        for (p, out) in [(a, scale(d, -1.0)), (b, d)] {
            let pts: Vec<[f32; 3]> = ring.iter().map(|&(s, t)| corner(p, s, t)).collect();
            self.face(&pts, &[[0.0, 0.0]; 4], out, 0, style);
        }
    }

    /// A box turned to `axis` about `c` (half lengths along and across), from `y0` to its top at `y1` (no floor).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn block(
        &mut self,
        c: [f32; 2],
        axis: [f32; 2],
        half: [f32; 2],
        y0: f32,
        y1: f32,
        style: [u8; 4],
        top: [u8; 4],
    ) {
        let n = [-axis[1], axis[0]];
        let at = |u: f32, v: f32| {
            [
                c[0] + axis[0] * u * half[0] + n[0] * v * half[1],
                c[1] + axis[1] * u * half[0] + n[1] * v * half[1],
            ]
        };
        let ring = [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (ring[i], ring[(i + 1) % 4]);
            let mid = [(a[0] + b[0]) * 0.5 - c[0], 0.0, (a[1] + b[1]) * 0.5 - c[1]];
            let len = super::geometry::distance(a, b);
            self.face(
                &[
                    [a[0], y0, a[1]],
                    [a[0], y1, a[1]],
                    [b[0], y1, b[1]],
                    [b[0], y0, b[1]],
                ],
                &[[0.0, 0.0], [0.0, y1 - y0], [len, y1 - y0], [len, 0.0]],
                mid,
                0,
                style,
            );
        }
        let pts: Vec<[f32; 3]> = ring.iter().map(|p| [p[0], y1, p[1]]).collect();
        let facade: Vec<[f32; 2]> = ring.iter().map(|p| [p[0] - c[0], p[1] - c[1]]).collect();
        self.face(&pts, &facade, [0.0, 1.0, 0.0], 0, top);
    }

    /// One of a building's extras, at its level of detail (heights over its mean ground; walls from below its lowest).
    pub(crate) fn extra(&mut self, b: &Building, e: &Extra, lod: Lod) {
        let g = b.ground[1];
        let bottom = super::mesh::foot_of(b);
        let full = lod == Lod::Full;
        match *e {
            Extra::Tower {
                centre,
                axis,
                half,
                top,
                cap,
            } => {
                let stages = ((top / 7.0).round() as i32).clamp(2, 40);
                let stage_m = top / stages as f32;
                let style = style_of(
                    b.wall,
                    b.roof_colour,
                    b.seed,
                    Kind::Church,
                    PART_WALL,
                    false,
                );
                let n = [-axis[1], axis[0]];
                let at = |u: f32, v: f32| {
                    [
                        centre[0] + (axis[0] * u + n[0] * v) * half,
                        centre[1] + (axis[1] * u + n[1] * v) * half,
                    ]
                };
                let ring = [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)];
                self.walls_with(
                    &ring,
                    bottom,
                    g + top,
                    g,
                    stage_m,
                    stages as i8,
                    2.0 * half / (2.0 * half / 3.5).round().max(1.0),
                    style,
                );
                let roof = style_of(
                    b.wall,
                    b.roof_colour,
                    b.seed,
                    Kind::Church,
                    PART_ROOF,
                    false,
                );
                self.flat_ring(&ring, g + top, roof);
                let phase = axis[1].atan2(axis[0]) + std::f32::consts::FRAC_PI_4;
                let corner = half * std::f32::consts::SQRT_2;
                match cap {
                    Cap::Spire { height, colour } => {
                        let style =
                            style_of(b.wall, colour, b.seed, Kind::Church, PART_DOME, false);
                        if full {
                            // An octagonal spire on the square tower, its corners at the sides' middles.
                            let axis_phase = axis[1].atan2(axis[0]);
                            self.cone(
                                centre,
                                half,
                                g + top,
                                g + top + height,
                                8,
                                axis_phase,
                                false,
                                style,
                            );
                        } else {
                            self.cone(
                                centre,
                                corner,
                                g + top,
                                g + top + height,
                                4,
                                phase,
                                false,
                                style,
                            );
                        }
                    }
                    Cap::Pyramid { height, colour } => {
                        let style =
                            style_of(b.wall, colour, b.seed, Kind::Church, PART_ROOF, false);
                        self.cone(
                            centre,
                            corner * 1.08,
                            g + top,
                            g + top + height,
                            4,
                            phase,
                            false,
                            style,
                        );
                    }
                    Cap::Flat => {}
                }
            }
            Extra::Dome {
                centre,
                radius,
                base,
                colour,
            } => {
                if !full {
                    return;
                }
                let drum = 0.35 * radius;
                let wall = style_of(
                    b.wall,
                    b.roof_colour,
                    b.seed,
                    Kind::Church,
                    PART_GABLE,
                    false,
                );
                let plain = |y: f32| y;
                let facade = Facade {
                    around: 12.0,
                    rows_at: &plain,
                    rows: 0,
                };
                self.prism(
                    centre,
                    radius,
                    radius,
                    g + base - 0.5,
                    g + base + drum,
                    16,
                    0.0,
                    true,
                    &facade,
                    wall,
                );
                let dome = style_of(b.wall, colour, b.seed, Kind::Church, PART_DOME, false);
                self.dome(centre, radius, g + base + drum, radius * 0.95, 16, 4, dome);
                // The lantern's finial.
                self.cone(
                    centre,
                    radius * 0.12,
                    g + base + drum + radius * 0.9,
                    g + base + drum + radius * 1.25,
                    6,
                    0.0,
                    true,
                    dome,
                );
            }
            Extra::Minaret {
                centre,
                radius,
                top,
                colour,
            } => {
                let wall = style_of(
                    b.wall,
                    b.roof_colour,
                    b.seed,
                    Kind::Church,
                    PART_GABLE,
                    false,
                );
                let cap = style_of(b.wall, colour, b.seed, Kind::Church, PART_DOME, false);
                let plain = |y: f32| y;
                let facade = Facade {
                    around: 8.0,
                    rows_at: &plain,
                    rows: 0,
                };
                if !full {
                    self.prism(
                        centre,
                        radius,
                        radius * 0.85,
                        bottom,
                        g + top,
                        4,
                        0.4,
                        false,
                        &facade,
                        wall,
                    );
                    return;
                }
                let gallery = g + top * 0.78;
                self.prism(
                    centre,
                    radius,
                    radius * 0.85,
                    bottom,
                    gallery,
                    10,
                    0.0,
                    true,
                    &facade,
                    wall,
                );
                // The balcony: a wider ring, its floor and its rail.
                let rb = radius * 1.55;
                self.disc(centre, rb, gallery, 10, 0.0, true, wall);
                self.prism(
                    centre,
                    rb,
                    rb,
                    gallery,
                    gallery + 1.0,
                    10,
                    0.0,
                    true,
                    &facade,
                    wall,
                );
                self.disc(centre, rb, gallery + 1.0, 10, 0.0, false, wall);
                let upper = g + top * 0.92;
                self.prism(
                    centre,
                    radius * 0.8,
                    radius * 0.78,
                    gallery + 1.0,
                    upper,
                    10,
                    0.0,
                    true,
                    &facade,
                    wall,
                );
                self.cone(
                    centre,
                    radius * 0.9,
                    upper,
                    g + top + top * 0.12,
                    10,
                    0.0,
                    true,
                    cap,
                );
            }
            Extra::Chimney {
                centre,
                radius,
                top,
                banded,
            } => {
                let body = super::pick_entry(&[4, 2, 3, 5, 2], b.seed);
                let style = style_of(body, 4, b.seed, Kind::Chimney, PART_BANDED, false);
                let band_m = (top * 0.04).max(2.0);
                let start = top * 0.72;
                let rows_at = move |y: f32| {
                    if banded {
                        (y - g - start) / band_m
                    } else {
                        -1.0
                    }
                };
                let facade = Facade {
                    around: 10.0,
                    rows_at: &rows_at,
                    rows: 0,
                };
                let (sides, smooth) = if full { (10, true) } else { (4, false) };
                self.prism(
                    centre,
                    radius,
                    radius * 0.7,
                    bottom,
                    g + top,
                    sides,
                    0.0,
                    smooth,
                    &facade,
                    style,
                );
                let soot = style_of(body, 12, b.seed, Kind::Chimney, PART_ROOF, false);
                self.disc(centre, radius * 0.7, g + top, sides, 0.0, false, soot);
            }
            Extra::Light {
                centre,
                radius,
                top,
                bands,
                red,
            } => {
                let band = if red { 4 } else { 12 };
                let body = style_of(13, band, b.seed, Kind::Lighthouse, PART_BANDED, false);
                let band_m = top / (2.0 * f32::from(bands.max(1)));
                let rows_at = move |y: f32| {
                    if bands > 0 {
                        (y - (g - 0.0)) / band_m
                    } else {
                        -1.0
                    }
                };
                let facade = Facade {
                    around: 12.0,
                    rows_at: &rows_at,
                    rows: 0,
                };
                let r1 = radius * 0.75;
                let lr = (r1 * 0.85).max(0.9);
                let lantern = 1.6 + 0.06 * top;
                if !full {
                    self.prism(
                        centre,
                        radius,
                        r1,
                        bottom,
                        g + top + lantern,
                        6,
                        0.0,
                        false,
                        &facade,
                        body,
                    );
                    return;
                }
                self.prism(
                    centre,
                    radius,
                    r1,
                    bottom,
                    g + top,
                    12,
                    0.0,
                    true,
                    &facade,
                    body,
                );
                // The gallery, its rail dark.
                let rg = (r1 + 0.8).min(radius);
                let white = style_of(13, band, b.seed, Kind::Lighthouse, PART_GABLE, false);
                let rail = style_of(
                    cube(40, 40, 40),
                    band,
                    b.seed,
                    Kind::Lighthouse,
                    PART_STEEL,
                    false,
                );
                self.disc(centre, rg, g + top - 0.35, 12, 0.0, true, white);
                self.prism(
                    centre,
                    rg,
                    rg,
                    g + top - 0.35,
                    g + top,
                    12,
                    0.0,
                    true,
                    &facade,
                    white,
                );
                self.disc(centre, rg, g + top, 12, 0.0, false, white);
                let plain = |y: f32| y;
                let thin = Facade {
                    around: 12.0,
                    rows_at: &plain,
                    rows: 0,
                };
                self.prism(
                    centre,
                    rg,
                    rg,
                    g + top,
                    g + top + 0.9,
                    12,
                    0.0,
                    true,
                    &thin,
                    rail,
                );
                // The lantern, glazed and lit at night, under its cap.
                let glass = style_of(13, band, b.seed, Kind::Lighthouse, PART_GLASS, true);
                self.prism(
                    centre,
                    lr,
                    lr,
                    g + top,
                    g + top + lantern,
                    10,
                    0.0,
                    true,
                    &thin,
                    glass,
                );
                let cap = style_of(13, band, b.seed, Kind::Lighthouse, PART_DOME, false);
                self.cone(
                    centre,
                    lr * 1.12,
                    g + top + lantern,
                    g + top + lantern + lr * 0.9,
                    10,
                    0.0,
                    true,
                    cap,
                );
            }
            Extra::Crane {
                at,
                out,
                gantry,
                colour,
            } => {
                if !full {
                    return;
                }
                let steel = style_of(colour, colour, b.seed, Kind::Crane, PART_STEEL, false);
                let dark = style_of(
                    cube(60, 60, 64),
                    colour,
                    b.seed,
                    Kind::Crane,
                    PART_STEEL,
                    false,
                );
                let along = [-out[1], out[0]];
                let p = |u: f32, v: f32, y: f32| {
                    [
                        at[0] + out[0] * u + along[0] * v,
                        g + y,
                        at[1] + out[1] * u + along[1] * v,
                    ]
                };
                if gantry {
                    // A container crane: four legs straddling the quay's rails, portal beams, the boom out over the
                    // water and back over the quay, the machinery house, the apex and its stays.
                    let (front, back, side, deck) = (9.0, -21.0, 9.0, 36.0);
                    for (u, v) in [(front, -side), (front, side), (back, -side), (back, side)] {
                        self.beam(p(u, v, -0.5), p(u, v, deck), 1.5, steel);
                    }
                    for v in [-side, side] {
                        self.beam(p(back, v, deck), p(front, v, deck), 1.6, steel);
                        self.beam(p(back, v, 11.0), p(front, v, 11.0), 1.0, steel);
                    }
                    for u in [front, back] {
                        self.beam(p(u, -side, deck + 1.0), p(u, side, deck + 1.0), 1.4, steel);
                    }
                    self.beam(
                        p(back - 18.0, 0.0, deck + 3.0),
                        p(front + 42.0, 0.0, deck + 3.0),
                        3.2,
                        steel,
                    );
                    self.block(
                        [p(back - 8.0, 0.0, 0.0)[0], p(back - 8.0, 0.0, 0.0)[2]],
                        out,
                        [6.0, 3.5],
                        g + deck + 4.6,
                        g + deck + 9.0,
                        dark,
                        dark,
                    );
                    let apex = p(back + 2.0, 0.0, deck + 22.0);
                    for v in [-side * 0.6, side * 0.6] {
                        self.beam(p(back, v, deck + 1.0), apex, 1.0, steel);
                        self.beam(p(front, v, deck + 1.0), apex, 1.0, steel);
                    }
                    self.beam(apex, p(front + 40.0, 0.0, deck + 4.5), 0.5, steel);
                } else {
                    // A jib crane: a portal on four legs, its turntable house, the A-frame and the jib raised out over
                    // the water.
                    let (half, portal) = (3.8, 9.0);
                    for (u, v) in [(-half, -half), (-half, half), (half, -half), (half, half)] {
                        self.beam(p(u, v, -0.5), p(u, v, portal), 0.8, steel);
                    }
                    self.block(
                        [at[0], at[1]],
                        out,
                        [half + 0.4, half + 0.4],
                        g + portal,
                        g + portal + 1.4,
                        steel,
                        steel,
                    );
                    let house = [at[0] - out[0] * 1.0, at[1] - out[1] * 1.0];
                    self.block(
                        house,
                        out,
                        [3.2, 2.3],
                        g + portal + 1.4,
                        g + portal + 5.2,
                        dark,
                        steel,
                    );
                    let heel = p(1.5, 0.0, portal + 4.0);
                    let tip = p(28.0, 0.0, portal + 4.0 + 18.0);
                    self.beam(heel, tip, 1.0, steel);
                    let mast = p(-1.0, 0.0, portal + 12.0);
                    self.beam(p(-1.0, 0.0, portal + 5.2), mast, 1.2, steel);
                    self.beam(mast, tip, 0.25, dark);
                    self.block(
                        [at[0] - out[0] * 4.5, at[1] - out[1] * 4.5],
                        out,
                        [1.2, 1.8],
                        g + portal + 1.4,
                        g + portal + 4.2,
                        dark,
                        dark,
                    );
                }
            }
        }
    }
}

/// The colour cube's entry for an sRGB colour.
fn cube(r: u8, g: u8, b: u8) -> u8 {
    super::cube_entry([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::super::mesh::{ChunkMesh, PART_BANDED, PART_GLASS};
    use super::super::{Building, Extra, Kind, Roof, geometry};
    use super::*;

    fn normal(n: [i8; 4]) -> [f32; 3] {
        [
            f32::from(n[0]) / 127.0,
            f32::from(n[1]) / 127.0,
            f32::from(n[2]) / 127.0,
        ]
    }

    /// Every triangle wound counter-clockwise seen from where its vertices' normals point.
    fn faces_out(mesh: &ChunkMesh) {
        for t in mesh.indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
            let n = cross(sub(b.position, a.position), sub(c.position, a.position));
            let stored = normal(a.normal);
            let mean = add(add(stored, normal(b.normal)), normal(c.normal));
            if (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) < 1e-10 {
                continue;
            }
            assert!(
                dot(n, mean) > 0.0,
                "a triangle facing in: {t:?} {n:?} {mean:?}"
            );
        }
    }

    /// Every vertex's normal points away from the axis through `c` (a round shape's outside), or up.
    fn normals_out(mesh: &ChunkMesh, c: [f32; 2]) {
        for v in &mesh.vertices {
            let n = normal(v.normal);
            let out = [v.position[0] - c[0], v.position[2] - c[1]];
            let radial = n[0] * out[0] + n[2] * out[1];
            assert!(radial > -1e-3 || n[1] > 0.5, "{:?} {n:?}", v.position);
        }
    }

    fn base(kind: Kind, ring: Vec<[f32; 2]>) -> Building {
        Building {
            ring,
            kind,
            roof: Roof::Flat,
            ground: [2.0, 3.0, 4.0],
            height: 10.0,
            base: 0.0,
            storeys: 3,
            storey_m: 3.0,
            wall: 1,
            roof_colour: 2,
            seed: 9,
            detail: false,
            bay: 3.0,
            extras: Vec::new(),
            period: crate::buildings::Period::Today,
        }
    }

    #[test]
    fn round_parts_close_with_their_normals_out() {
        let c = [50.0, 60.0];
        let mut m = MeshBuilder::default();
        let rows = |y: f32| y;
        let facade = Facade {
            around: 10.0,
            rows_at: &rows,
            rows: 0,
        };
        let style = style_of(1, 2, 3, Kind::Tank, PART_GABLE, false);
        m.prism(c, 10.0, 8.0, 0.0, 12.0, 16, 0.2, true, &facade, style);
        m.cone(c, 8.0, 12.0, 15.0, 16, 0.2, true, style);
        m.dome(c, 6.0, 15.0, 5.0, 12, 4, style);
        m.disc(c, 10.0, 0.0, 16, 0.2, true, style);
        let mesh = m.finish();
        faces_out(&mesh);
        // Within the footprint's circle and up to the dome's crown.
        for v in &mesh.vertices {
            let r = (v.position[0] - c[0]).hypot(v.position[2] - c[1]);
            assert!(r <= 10.0 + 1e-3 && v.position[1] <= 20.0 + 1e-3);
        }
        assert_eq!(mesh.triangles(), 16 * 2 + 16 + 12 * 3 * 2 + 12 + 14);
        let mut m = MeshBuilder::default();
        m.prism(c, 10.0, 8.0, 0.0, 12.0, 16, 0.2, true, &facade, style);
        m.cone(c, 8.0, 12.0, 15.0, 16, 0.2, true, style);
        normals_out(&m.finish(), c);
        // A beam and a box, faces out.
        let mut m = MeshBuilder::default();
        m.beam([0.0, 0.0, 0.0], [10.0, 8.0, 3.0], 1.0, style);
        m.block([5.0, 5.0], [0.6, 0.8], [4.0, 2.0], 0.0, 6.0, style, style);
        let mesh = m.finish();
        faces_out(&mesh);
        assert_eq!(mesh.triangles(), 6 * 2 + 5 * 2);
    }

    #[test]
    fn extras_stand_within_their_footprints() {
        // A church: its tower at the west end within the nave's footprint, its spire over it.
        let nave = vec![
            [100.0, 100.0],
            [136.0, 100.0],
            [136.0, 116.0],
            [100.0, 116.0],
        ];
        let mut church = base(Kind::Church, nave.clone());
        church.extras.push(Extra::Tower {
            centre: [104.5, 108.0],
            axis: [1.0, 0.0],
            half: 4.5,
            top: 30.0,
            cap: Cap::Spire {
                height: 20.0,
                colour: 21,
            },
        });
        church.extras.push(Extra::Dome {
            centre: [125.0, 108.0],
            radius: 5.0,
            base: 14.0,
            colour: 22,
        });
        for lod in [Lod::Full, Lod::Box] {
            let mut m = MeshBuilder::default();
            for e in &church.extras {
                m.extra(&church, e, lod);
            }
            let mesh = m.finish();
            faces_out(&mesh);
            for v in &mesh.vertices {
                let p = [v.position[0], v.position[2]];
                assert!(
                    geometry::contains(&nave, p) || geometry::ring_distance(&nave, p) < 1e-3,
                    "{p:?} outside the nave"
                );
            }
            // The spire's tip at the tower's top plus its height over the mean ground (3 m).
            let top = mesh
                .vertices
                .iter()
                .map(|v| v.position[1])
                .fold(0.0, f32::max);
            assert!((top - 53.0).abs() < 1e-3, "{top}");
        }
        // A lighthouse: banded tower, a glazed lantern, all within a few metres of its middle.
        let mut light = base(
            Kind::Lighthouse,
            geometry::regular([10.0, 10.0], 3.0, 12, 0.0),
        );
        light.extras.push(Extra::Light {
            centre: [10.0, 10.0],
            radius: 3.0,
            top: 24.0,
            bands: 3,
            red: true,
        });
        let mut m = MeshBuilder::default();
        m.extra(&light, &light.extras[0], Lod::Full);
        let mesh = m.finish();
        faces_out(&mesh);
        assert!(
            mesh.vertices
                .iter()
                .any(|v| (v.style[3] >> 4) & 7 == PART_GLASS)
        );
        assert!(
            mesh.vertices
                .iter()
                .any(|v| (v.style[3] >> 4) & 7 == PART_BANDED)
        );
        for v in &mesh.vertices {
            assert!((v.position[0] - 10.0).hypot(v.position[2] - 10.0) <= 3.0 + 1e-3);
        }
        // A crane and a minaret, faces out; the crane is dropped from a box chunk.
        let mut quay = base(Kind::Crane, geometry::regular([0.0, 0.0], 4.0, 4, 0.0));
        quay.extras.push(Extra::Crane {
            at: [0.0, 0.0],
            out: [1.0, 0.0],
            gantry: true,
            colour: 40,
        });
        quay.extras.push(Extra::Crane {
            at: [0.0, 50.0],
            out: [0.0, -1.0],
            gantry: false,
            colour: 41,
        });
        quay.extras.push(Extra::Minaret {
            centre: [100.0, 0.0],
            radius: 1.8,
            top: 40.0,
            colour: 22,
        });
        let mut m = MeshBuilder::default();
        for e in &quay.extras {
            m.extra(&quay, e, Lod::Full);
        }
        let mesh = m.finish();
        faces_out(&mesh);
        assert!(mesh.triangles() > 200);
        let mut m = MeshBuilder::default();
        m.extra(&quay, &quay.extras[0], Lod::Box);
        assert_eq!(m.finish().triangles(), 0);
    }
}
