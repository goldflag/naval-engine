//! Airfields laid on the ground (native/docs/terrain/infra.md): runways, taxiways and aprons from
//! OpenMapTiles' `aeroway` layer, draped over the ground as decals ([`Group::Decal`]: pulled toward the eye, casting
//! nothing) and marked by the shader (`infra.wgsl`'s runway, taxiway and apron materials).
//!
//! **Runways** come whole (the host joins a runway's pieces across tiles), so both ends are known: each half is drawn
//! from its end, with that end's designator (OSM's `ref`, `07L/25R`, matched to the ends by their headings), the
//! threshold's bars, the aiming point and touchdown zone (on runways of 1,800 m and more), the dashed centre line and
//! the side stripes, the FAA's sizes (a 45 m wide runway's). OpenMapTiles carries no runway width: a runway's outline
//! where mapped gives it, else its length (45 m from 1,800 m, 30 m from 1,000 m, else 23 m). **Taxiways** are strips
//! along their lines (23 m wide at an airport with a long runway, else 15 m) with a yellow centre line and edge line;
//! **aprons** (and taxiway, runway and helipad areas) are concrete with its joints. Heights: aprons under taxiways under
//! runways, a few centimetres apart, so overlaps never fight.

use glam::{Vec2, Vec3};

use super::mesh::{Group, Material, MeshBuilder, Paint};
use super::period::RunwayPaint;
use crate::buildings::{CHUNK_M, geometry};

/// A chunk's side (m).
const SIDE: f32 = CHUNK_M as f32;

/// A runway's designator at one end: its number (1 to 36) and letter (`L`, `C`, `R` or none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Designator {
    pub number: u8,
    pub letter: Option<char>,
}

impl Designator {
    /// One designator as OSM writes it (`07L`, `25`, `7R`), none if it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        let number: u8 = digits.parse().ok()?;
        if !(1..=36).contains(&number) {
            return None;
        }
        let rest = text[digits.len()..].trim();
        let letter = match rest {
            "" => None,
            "L" | "l" => Some('L'),
            "C" | "c" => Some('C'),
            "R" | "r" => Some('R'),
            _ => return None,
        };
        Some(Self { number, letter })
    }

    /// The designator a heading (degrees clockwise from true north) rounds to.
    pub fn of_heading(heading: f32) -> Self {
        let n = (heading.rem_euclid(360.0) / 10.0).round() as u8;
        Self {
            number: if n == 0 { 36 } else { n.min(36) },
            letter: None,
        }
    }

    fn letter_code(&self) -> u8 {
        match self.letter {
            Some('L') => 1,
            Some('C') => 2,
            Some('R') => 3,
            _ => 0,
        }
    }
}

/// A runway: its centre line whole (chunk-local metres, as the host joined it), OSM's `ref`, and its paved half width
/// (m; [`Runway::half_for`] when the map gives none).
#[derive(Clone, Debug, PartialEq)]
pub struct Runway {
    pub points: Vec<Vec2>,
    pub reference: Option<String>,
    pub half: f32,
}

impl Runway {
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// A runway's half width by its length when no outline says.
    pub fn half_for(length: f32) -> f32 {
        if length >= 1800.0 {
            22.5
        } else if length >= 1000.0 {
            15.0
        } else {
            11.5
        }
    }

    /// The heading (degrees clockwise from true north) from end `end` (0: the first point) along the runway.
    pub fn heading(&self, end: usize) -> f32 {
        let (a, b) = (
            self.points[0],
            *self.points.last().expect("a runway's points"),
        );
        let d = if end == 0 { b - a } else { a - b };
        // x east, z south: north is −z.
        d.x.atan2(-d.y).to_degrees().rem_euclid(360.0)
    }

    /// The designators painted at each end: OSM's two matched to the ends by heading (magnetic north is within some
    /// degrees of true; the two differ by 180°), else each end's heading rounded.
    pub fn designators(&self) -> [Designator; 2] {
        let headings = [self.heading(0), self.heading(1)];
        let own = headings.map(Designator::of_heading);
        let Some(reference) = &self.reference else {
            return own;
        };
        let parts: Vec<Designator> = reference
            .split(['/', ';', '-'])
            .filter_map(Designator::parse)
            .collect();
        let [a, b] = match parts.as_slice() {
            [a, b] => [*a, *b],
            _ => return own,
        };
        let off = |d: Designator, h: f32| {
            let x = (f32::from(d.number) * 10.0 - h).rem_euclid(360.0);
            x.min(360.0 - x)
        };
        if off(a, headings[0]) + off(b, headings[1]) <= off(b, headings[0]) + off(a, headings[1]) {
            [a, b]
        } else {
            [b, a]
        }
    }
}

/// A taxiway's line (chunk-local, its tile's piece) and OSM's `ref` (its letter), which sets its height among the
/// taxiways crossing it.
#[derive(Clone, Debug, PartialEq)]
pub struct Taxiway {
    pub points: Vec<Vec2>,
    pub reference: Option<String>,
}

/// A paved area: an apron, or a taxiway's, runway's or helipad's area (chunk-local exterior ring, its tile's piece).
#[derive(Clone, Debug, PartialEq)]
pub struct Apron {
    pub ring: Vec<[f32; 2]>,
    /// A runway's area (a past period draws its runways from their capped lines instead: `infra::period`).
    pub runway: bool,
}

/// A runway as drawn, for its lights (`InfraLayer::runways`): its thresholds (x, z: chunk-local in a mesh, theatre
/// metres from the layer; y the paint's height, metres above the sea) and its paved half width (m), the designators at
/// each end.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct RunwaySpan {
    pub ends: [[f64; 3]; 2],
    pub half: f32,
    pub designators: [(u8, Option<char>); 2],
}

impl RunwaySpan {
    /// The same span moved by `by` (x, z).
    pub fn moved(&self, by: [f64; 2]) -> Self {
        Self {
            ends: self.ends.map(|e| [e[0] + by[0], e[1], e[2] + by[1]]),
            ..*self
        }
    }
}

/// An airfield's parts about a chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Airfield {
    pub runways: Vec<Runway>,
    pub taxiways: Vec<Taxiway>,
    pub aprons: Vec<Apron>,
    /// Whether runway numbers keep their leading zero (`07`, ICAO's; the FAA paints `7`).
    pub leading_zero: bool,
    /// The length of the longest runway about the chunk, those not passing it included (m): the airport's, which sets
    /// its taxiways' width alike in every chunk.
    pub longest_runway: f32,
    /// Painted as in a past period (the host's `period_airfield`, [`super::period::runway_paint`]): plain, or
    /// numbered without a touchdown zone or aiming point; today's marks when none.
    pub paint: Option<RunwayPaint>,
}

impl Airfield {
    pub fn is_empty(&self) -> bool {
        self.runways.is_empty() && self.taxiways.is_empty() && self.aprons.is_empty()
    }

    /// A taxiway's half width: 11.5 m at an airport with a runway of 1,800 m or more, else 7.5 m.
    pub fn taxiway_half(&self) -> f32 {
        let longest = self
            .runways
            .iter()
            .map(Runway::length)
            .fold(self.longest_runway, f32::max);
        if longest >= 1800.0 { 11.5 } else { 7.5 }
    }
}

/// Heights over the ground (m): aprons under taxiways under runways.
const APRON_LIFT: f32 = 0.10;
const TAXIWAY_LIFT: f32 = 0.16;
const RUNWAY_LIFT: f32 = 0.26;
/// Steps along a runway's or taxiway's strip (m) and an apron's grid (m): the drape over the ground.
const RUNWAY_STEP: f32 = 30.0;
const TAXIWAY_STEP: f32 = 20.0;
const APRON_CELL: f32 = 40.0;
/// Bit flags in a runway vertex's blue byte.
const FLAG_LEADING_ZERO: u8 = 1;
const FLAG_TOUCHDOWN: u8 = 2;
/// Plain pavement, no marks (a wartime runway's).
const FLAG_PLAIN: u8 = 4;

fn owned(p: Vec2) -> bool {
    (0.0..SIDE).contains(&p.x) && (0.0..SIDE).contains(&p.y)
}

fn hash_text(text: Option<&str>) -> u32 {
    text.unwrap_or("").bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    })
}

/// Lays a chunk's airfield: the runways' quads whose middles the chunk holds, its taxiways' and its aprons' share.
/// Returns the runways it drew a part of.
pub fn draw(m: &mut MeshBuilder, field: &Airfield, ground: &dyn Fn(Vec2) -> f32) -> u32 {
    if field.is_empty() {
        return 0;
    }
    let group = m.group();
    m.set_group(Group::Decal);
    for apron in &field.aprons {
        apron_surface(m, &apron.ring, ground);
    }
    let half = field.taxiway_half();
    for taxiway in &field.taxiways {
        let rank = (hash_text(taxiway.reference.as_deref()) % 5) as f32;
        taxiway_strip(
            m,
            &taxiway.points,
            half,
            TAXIWAY_LIFT + 0.012 * rank,
            ground,
        );
    }
    let mut drawn = 0;
    for runway in &field.runways {
        if runway_strip(m, runway, field, ground) {
            drawn += 1;
            let (a, b) = (runway.points[0], runway.points[runway.points.len() - 1]);
            let end = |p: Vec2| {
                [
                    f64::from(p.x),
                    f64::from(ground(p) + RUNWAY_LIFT),
                    f64::from(p.y),
                ]
            };
            m.runways.push(RunwaySpan {
                ends: [end(a), end(b)],
                half: runway.half.clamp(8.0, 40.0),
                designators: runway.designators().map(|d| (d.number, d.letter)),
            });
        }
    }
    m.set_group(group);
    drawn
}

/// One runway's halves, each from its end; true if the chunk drew any of it.
fn runway_strip(
    m: &mut MeshBuilder,
    runway: &Runway,
    field: &Airfield,
    ground: &dyn Fn(Vec2) -> f32,
) -> bool {
    if runway.points.len() < 2 {
        return false;
    }
    let (a, b) = (runway.points[0], *runway.points.last().expect("two points"));
    let length = a.distance(b);
    if length < 100.0 {
        return false;
    }
    let half = runway.half.clamp(8.0, 40.0);
    let designators = runway.designators();
    let mut flags = if field.leading_zero {
        FLAG_LEADING_ZERO
    } else {
        0
    };
    match field.paint {
        None if length >= 1800.0 && half >= 18.0 => flags |= FLAG_TOUCHDOWN,
        Some(RunwayPaint::Plain) => flags |= FLAG_PLAIN,
        _ => {}
    }
    let mut any = false;
    for (end, designator) in designators.into_iter().enumerate() {
        let (from, to) = if end == 0 { (a, b) } else { (b, a) };
        let d = (to - from) / length;
        // The pilot's right, landing from this end.
        let right = Vec2::new(-d.y, d.x);
        let paint = Paint {
            colour: [designator.number, designator.letter_code(), flags],
            material: Material::Runway,
            param: half.round() as i8,
        };
        let mid = length * 0.5;
        let steps = (mid / RUNWAY_STEP).ceil() as usize;
        let across = [-1.0, -0.5, 0.0, 0.5, 1.0].map(|f| f * half);
        for k in 0..steps {
            let s0 = k as f32 * RUNWAY_STEP;
            let s1 = ((k + 1) as f32 * RUNWAY_STEP).min(mid);
            if !owned(from + d * (0.5 * (s0 + s1))) {
                continue;
            }
            any = true;
            let row = |s: f32| -> [(Vec3, Vec2); 5] {
                across.map(|c| {
                    let p = from + d * s + right * c;
                    (
                        Vec3::new(p.x, ground(p) + RUNWAY_LIFT, p.y),
                        Vec2::new(s, c),
                    )
                })
            };
            let (r0, r1) = (row(s0), row(s1));
            for j in 0..4 {
                m.quad(
                    [r0[j].0, r0[j + 1].0, r1[j + 1].0, r1[j].0],
                    [r0[j].1, r0[j + 1].1, r1[j + 1].1, r1[j].1],
                    Vec3::Y,
                    paint,
                );
            }
        }
    }
    any
}

/// A taxiway's strip along its line (its segments whose middles the chunk holds), mitred at its bends.
fn taxiway_strip(
    m: &mut MeshBuilder,
    points: &[Vec2],
    half: f32,
    lift: f32,
    ground: &dyn Fn(Vec2) -> f32,
) {
    // Resampled every few metres, so the strip follows the ground.
    let mut line: Vec<Vec2> = Vec::new();
    for w in points.windows(2) {
        let n = (w[0].distance(w[1]) / TAXIWAY_STEP).ceil().max(1.0) as usize;
        for k in 0..n {
            line.push(w[0].lerp(w[1], k as f32 / n as f32));
        }
    }
    if let Some(last) = points.last() {
        line.push(*last);
    }
    line.dedup_by(|a, b| a.distance(*b) < 0.05);
    if line.len() < 2 {
        return;
    }
    let dirs: Vec<Vec2> = line
        .windows(2)
        .map(|w| (w[1] - w[0]).normalize_or_zero())
        .collect();
    // Each point's offset across: the mitre of its two segments, held to twice the half width.
    let offsets: Vec<Vec2> = (0..line.len())
        .map(|i| {
            let before = dirs[i.saturating_sub(1).min(dirs.len() - 1)];
            let after = dirs[i.min(dirs.len() - 1)];
            let r = |d: Vec2| Vec2::new(-d.y, d.x);
            let mitre = (r(before) + r(after)).normalize_or(r(after));
            let cos = mitre.dot(r(after)).max(0.5);
            mitre * (half / cos)
        })
        .collect();
    let paint = Paint {
        colour: [0, 0, 0],
        material: Material::Taxiway,
        param: half.round() as i8,
    };
    let mut along = 0.0f32;
    for i in 0..line.len() - 1 {
        let length = line[i].distance(line[i + 1]);
        let s0 = along;
        along += length;
        if !owned((line[i] + line[i + 1]) * 0.5) {
            continue;
        }
        // Along wraps every 512 m (the marks need only the metre's fraction).
        let (u0, u1) = (s0 % 512.0, s0 % 512.0 + length);
        let at = |p: Vec2| Vec3::new(p.x, ground(p) + lift, p.y);
        let rows = [
            [
                (line[i] - offsets[i], -half),
                (line[i], 0.0),
                (line[i] + offsets[i], half),
            ],
            [
                (line[i + 1] - offsets[i + 1], -half),
                (line[i + 1], 0.0),
                (line[i + 1] + offsets[i + 1], half),
            ],
        ];
        for j in 0..2 {
            let (p00, c00) = rows[0][j];
            let (p01, c01) = rows[0][j + 1];
            let (p10, c10) = rows[1][j];
            let (p11, c11) = rows[1][j + 1];
            m.quad(
                [at(p00), at(p01), at(p11), at(p10)],
                [
                    Vec2::new(u0, c00),
                    Vec2::new(u0, c01),
                    Vec2::new(u1, c11),
                    Vec2::new(u1, c10),
                ],
                Vec3::Y,
                paint,
            );
        }
    }
}

/// An apron's share of the chunk: the ring cut into the cells of a grid and each piece laid on the ground.
fn apron_surface(m: &mut MeshBuilder, ring: &[[f32; 2]], ground: &dyn Fn(Vec2) -> f32) {
    if ring.len() < 3 {
        return;
    }
    let mut ring = ring.to_vec();
    if geometry::signed_area2(&ring) < 0.0 {
        ring.reverse();
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &ring {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let lo = [lo[0].max(0.0), lo[1].max(0.0)];
    let hi = [hi[0].min(SIDE), hi[1].min(SIDE)];
    if lo[0] >= hi[0] || lo[1] >= hi[1] {
        return;
    }
    let paint = Paint::new([150, 150, 144], Material::Apron);
    let cells = |a: f32, b: f32| {
        let (i0, i1) = (
            (a / APRON_CELL).floor() as i32,
            (b / APRON_CELL).ceil() as i32,
        );
        i0..i1
    };
    for j in cells(lo[1], hi[1]) {
        for i in cells(lo[0], hi[0]) {
            let (x0, z0) = (i as f32 * APRON_CELL, j as f32 * APRON_CELL);
            let cell = [
                [x0, z0],
                [x0 + APRON_CELL, z0],
                [x0 + APRON_CELL, z0 + APRON_CELL],
                [x0, z0 + APRON_CELL],
            ];
            let piece = geometry::clip_polygon(&ring, &cell);
            if piece.len() < 3 || geometry::area(&piece) < 0.5 {
                continue;
            }
            let points: Vec<Vec3> = piece
                .iter()
                .map(|p| {
                    let q = Vec2::from(*p);
                    Vec3::new(q.x, ground(q) + APRON_LIFT, q.y)
                })
                .collect();
            let uv: Vec<Vec2> = piece.iter().map(|_| Vec2::ZERO).collect();
            for t in geometry::triangulate(&piece) {
                let ix = t.map(|k| k as usize);
                m.face(
                    &[points[ix[0]], points[ix[1]], points[ix[2]]],
                    &[uv[ix[0]], uv[ix[1]], uv[ix[2]]],
                    Vec3::Y,
                    paint,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mesh::Group;

    fn lax() -> Runway {
        // 07L/25R: west-south-west to east-north-east, 3,000 m, across the chunk and beyond it.
        let d = Vec2::new(83f32.to_radians().sin(), -83f32.to_radians().cos());
        let a = Vec2::new(-1000.0, 600.0);
        Runway {
            points: vec![a, a + d * 3000.0],
            reference: Some("07L/25R".into()),
            half: Runway::half_for(3000.0),
        }
    }

    #[test]
    fn designators_parse_and_match_their_ends() {
        assert_eq!(
            Designator::parse("07L"),
            Some(Designator {
                number: 7,
                letter: Some('L')
            })
        );
        assert_eq!(
            Designator::parse("25"),
            Some(Designator {
                number: 25,
                letter: None
            })
        );
        assert_eq!(Designator::parse("H1"), None);
        assert_eq!(Designator::parse("40"), None);
        let runway = lax();
        assert!((runway.heading(0) - 83.0).abs() < 0.5);
        let [a, b] = runway.designators();
        assert_eq!(
            (a.number, a.letter, b.number, b.letter),
            (7, Some('L'), 25, Some('R'))
        );
        // Written the other way round, still matched to the ends.
        let swapped = Runway {
            reference: Some("25R/07L".into()),
            ..runway.clone()
        };
        assert_eq!(swapped.designators(), [a, b]);
        let unnamed = Runway {
            reference: None,
            ..runway
        };
        assert_eq!(unnamed.designators().map(|d| d.number), [8, 26]);
    }

    #[test]
    fn a_runway_is_laid_once_across_the_chunks_it_crosses() {
        let field = Airfield {
            runways: vec![lax()],
            ..Airfield::default()
        };
        let ground = |_: Vec2| 30.0;
        let mut area = 0.0f32;
        let mut drawn_in = 0;
        for i in -2..4 {
            for j in -2..2 {
                let corner = Vec2::new(i as f32 * SIDE, j as f32 * SIDE);
                let local = Airfield {
                    runways: field
                        .runways
                        .iter()
                        .map(|r| Runway {
                            points: r.points.iter().map(|p| *p - corner).collect(),
                            ..r.clone()
                        })
                        .collect(),
                    ..Airfield::default()
                };
                let mut m = MeshBuilder::new(true);
                let n = draw(&mut m, &local, &ground);
                let mesh = m.finish();
                if n > 0 {
                    drawn_in += 1;
                    assert!(!mesh.range(Group::Decal).is_empty());
                    assert!(mesh.range(Group::Structure).is_empty());
                    assert!(mesh.min[1] > 30.0 && mesh.max[1] < 30.5);
                }
                for t in mesh.indices[mesh.range(Group::Decal).start as usize..].chunks(3) {
                    let p = [0, 1, 2].map(|i| Vec3::from(mesh.vertices[t[i] as usize].position));
                    area += 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
                }
            }
        }
        // x from −1,000 to 1,977 m, z from 600 to 234 m: three chunks.
        assert_eq!(drawn_in, 3);
        let whole = 3000.0 * 45.0;
        assert!((area - whole).abs() < whole * 0.01, "{area} of {whole}");
    }

    #[test]
    fn taxiways_and_aprons_lie_under_the_runway_within_the_chunk() {
        let field = Airfield {
            runways: vec![Runway {
                points: vec![Vec2::new(100.0, 500.0), Vec2::new(900.0, 500.0)],
                reference: Some("09/27".into()),
                half: 15.0,
            }],
            taxiways: vec![Taxiway {
                points: vec![
                    Vec2::new(200.0, 400.0),
                    Vec2::new(500.0, 400.0),
                    Vec2::new(500.0, 1200.0),
                ],
                reference: Some("A".into()),
            }],
            aprons: vec![Apron {
                ring: vec![
                    [300.0, 700.0],
                    [1300.0, 700.0],
                    [1300.0, 900.0],
                    [300.0, 900.0],
                ],
                runway: false,
            }],
            leading_zero: true,
            longest_runway: 0.0,
            paint: None,
        };
        let mut m = MeshBuilder::new(true);
        assert_eq!(draw(&mut m, &field, &|_| 2.0), 1);
        let mesh = m.finish();
        let mut apron = 0.0f32;
        let (mut runway_y, mut taxiway_y, mut apron_y) = (0.0f32, 0.0f32, 0.0f32);
        for t in mesh.indices.chunks(3) {
            let v = [0, 1, 2].map(|i| mesh.vertices[t[i] as usize]);
            let p = v.map(|v| Vec3::from(v.position));
            assert!(
                p.iter()
                    .all(|p| p.z <= SIDE + RUNWAY_STEP && p.x <= SIDE + RUNWAY_STEP)
            );
            match v[0].colour[3] {
                m if m == Material::Runway as u8 => runway_y = p[0].y,
                m if m == Material::Taxiway as u8 => taxiway_y = p[0].y,
                m if m == Material::Apron as u8 => {
                    apron_y = p[0].y;
                    apron += 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
                }
                other => panic!("material {other}"),
            }
        }
        // The apron's share: x 300 to 1000 of it.
        assert!((apron - 700.0 * 200.0).abs() < 1.0, "{apron}");
        assert!(
            apron_y < taxiway_y && taxiway_y < runway_y,
            "{apron_y} {taxiway_y} {runway_y}"
        );
        // The taxiway's part beyond the chunk is another's.
        assert!(mesh.max[2] < SIDE + TAXIWAY_STEP);
    }

    #[test]
    fn a_past_periods_runway_has_its_paint() {
        // The flags byte of a 3,000 m runway's vertices, painted today, numbered (after 1948) and plain (the war's).
        let flags = |paint: Option<RunwayPaint>| {
            let field = Airfield {
                runways: vec![Runway {
                    points: vec![Vec2::new(-1000.0, 500.0), Vec2::new(2000.0, 500.0)],
                    reference: Some("09/27".into()),
                    half: 22.5,
                }],
                paint,
                ..Airfield::default()
            };
            let mut m = MeshBuilder::new(true);
            assert_eq!(draw(&mut m, &field, &|_| 2.0), 1);
            m.finish().vertices[0].colour[2]
        };
        assert_eq!(flags(None), FLAG_TOUCHDOWN);
        assert_eq!(flags(Some(RunwayPaint::Numbered)), 0);
        assert_eq!(flags(Some(RunwayPaint::Plain)), FLAG_PLAIN);
    }

    #[test]
    fn a_taxiway_is_as_wide_wherever_its_runways_lie() {
        // A chunk the long runway passes, and one beyond it that knows it only by its length.
        let near = Airfield {
            runways: vec![lax()],
            ..Airfield::default()
        };
        let beyond = Airfield {
            longest_runway: 3000.0,
            ..Airfield::default()
        };
        assert_eq!(near.taxiway_half(), 11.5);
        assert_eq!(beyond.taxiway_half(), 11.5);
        assert_eq!(Airfield::default().taxiway_half(), 7.5);
    }
}
