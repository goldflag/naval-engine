//! The `luftwaffe-field` template: a Luftwaffe airfield of 1943 in the Netherlands, Denmark, the German Bight or
//! Norway laid out on its site ([`luftwaffe_field`]), in metres about the base's point, in its farmland. A major base
//! (Fliegerhorst): a broad field of grass with one concrete runway in slabs, often a second crossing it near an end,
//! hedgerows painted across the grass as camouflage, concrete taxiways winding off into the country to dispersal
//! boxes (earth walls in a U, grassed over, the larger for bombers, some under the trees, a few netted), big hangars
//! in a row with their concrete apron, brick technical buildings, barracks built as a village of farmhouses (rendered
//! or brick walls, steep red tiled roofs) along a lane, a small control building, 2 cm Flak in earth rings about the
//! field and a battery of 8.8 cm guns. A field strip (Feldflugplatz): a mown run across a grass field, a few wooden
//! huts and tents, boxes at the field's edge under the trees, a little Flak. The field is cleared to mown grass (hard,
//! temperate); all else among the fields and woods only where it stands. Parks nothing of its own (the client has no
//! Luftwaffe type).

use std::f32::consts::TAU;

use glam::Vec2;

use super::ijn::{edge, winding};
use super::site::{Site, SiteInput};
use super::strip::{
    Capsule, Drums, Gun, Hut, HutKind, LOT_LIFT, Layout, Look, Net, Pad, Pit, Plan, RUNWAY_LIFT,
    Revetment, Ribbon, Surface, Tent, Vehicle, VehicleKind, distances, line_distance, perp,
    point_at, rotate, rough,
};
use super::{Rng, SizeClass};

/// Render and brick; red tiles; a hall's walls and roof; the grassed boxes' earth; field grey; tent canvas; huts'
/// planks; netting.
const RENDER: [[u8; 3]; 3] = [[216, 208, 190], [206, 198, 184], [196, 190, 176]];
const BRICK: [[u8; 3]; 2] = [[150, 78, 58], [138, 72, 56]];
const TILE: [[u8; 3]; 3] = [[150, 66, 46], [138, 60, 44], [160, 76, 54]];
const HALL_WALLS: [[u8; 3]; 2] = [[168, 164, 152], [150, 146, 136]];
const HALL_ROOF: [[u8; 3]; 2] = [[66, 68, 66], [78, 76, 72]];
const SOIL: [[u8; 3]; 2] = [[92, 96, 66], [100, 98, 70]];
const GREY: [[u8; 3]; 2] = [[78, 80, 80], [70, 72, 72]];
const CANVAS: [[u8; 3]; 2] = [[96, 100, 82], [88, 92, 76]];
const PLANKS: [[u8; 3]; 2] = [[86, 74, 58], [74, 66, 52]];
const NETTING: [[u8; 3]; 2] = [[72, 82, 52], [80, 86, 56]];

/// The painted hedges' height over the grass (m): under all else laid.
const HEDGE_LIFT: f32 = 0.03;

/// The field laid out on `site`, as `input` (its seed, size, ground, neighbours and catalog entry) allows.
pub fn luftwaffe_field(site: Site, input: &SiteInput<'_>) -> Plan {
    let major = input.size == SizeClass::BomberField;
    let mut rng = Rng::new(input.seed, 4);
    // No Luftwaffe type in the client: its boxes stand empty.
    let fill = rng.range(0.2, 0.35);
    let mut l = Layout::new(site, input, rng, !major, Vec::new(), fill);
    l.road_look = Look::Tarmac;
    if major {
        fliegerhorst(&mut l);
    } else {
        feldflugplatz(&mut l);
    }
    l.finish()
}

/// The field's grass about its runs: a capsule along it `across` either side, past its ends by as much.
fn field(l: &mut Layout<'_, '_>, across: f32) {
    let s = l.site;
    l.plan.clearing.push(Capsule {
        a: s.at(-0.5 * s.length + 0.6 * across, 0.0),
        b: s.at(0.5 * s.length - 0.6 * across, 0.0),
        radius: across,
        soft: false,
    });
    super::raf::bulges(l, Vec2::new(0.5 * s.length + 0.4 * across, across));
}

/// Hedgerows painted across the field's grass, along its old fields' lines (two ways at right angles, some 150-250 m
/// apart), broken where paving crosses them.
fn painted_hedges(l: &mut Layout<'_, '_>, reach: Vec2, count: usize) {
    let s = l.site;
    let grid = rotate(s.dir, l.rng.range(0.0, 90.0));
    for k in 0..count {
        let dir = if k % 2 == 0 { grid } else { perp(grid) };
        let offset = l.rng.signed() * 0.8 * reach.y.min(reach.x);
        let base = s.centre + perp(dir) * offset;
        let wobble = l.rng.range(2.0, 6.0);
        let phase = l.rng.range(0.0, TAU);
        let mut run: Vec<Vec2> = Vec::new();
        let mut t = -reach.x;
        while t <= reach.x {
            let p = base + dir * t + perp(dir) * (wobble * (t * 0.013 + phase).sin());
            let on = l.in_field(p, 15.0) && l.off_strips(p) > 3.0 && l.off_pieces(p) > 3.0;
            if on {
                run.push(p);
            } else if run.len() >= 3 {
                let points = std::mem::take(&mut run);
                lay_hedge(l, points);
            } else {
                run.clear();
            }
            t += 15.0;
        }
        if run.len() >= 3 {
            lay_hedge(l, run);
        }
    }
}

fn lay_hedge(l: &mut Layout<'_, '_>, points: Vec<Vec2>) {
    let half = l.rng.range(2.4, 3.4);
    l.plan.ribbons.push(Ribbon {
        points,
        half,
        paved: half,
        look: Look::Hedge,
        surface: Surface::Coral,
        lift: HEDGE_LIFT,
        across: 1,
    });
}

/// The Fliegerhorst.
fn fliegerhorst(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let [e0, e1] = s.ends();
    let across = l.rng.range(380.0, 480.0);
    field(l, across);
    l.plan.ribbons.push(Ribbon {
        points: vec![e0, e1],
        half: s.half,
        paved: s.half,
        look: Look::Concrete,
        surface: Surface::Coral,
        lift: RUNWAY_LIFT,
        across: 4,
    });
    // A second runway crossing it near an end, at 50-80°.
    if l.rng.chance(0.55) {
        let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let turn = l.rng.range(50.0, 80.0) * if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let d = rotate(s.dir, turn);
        let cross = s.at(sign * 0.28 * s.length, 0.0);
        let length = l.rng.range(1150.0, 1300.0);
        let (a, b) = (cross - d * (0.35 * length), cross + d * (0.65 * length));
        let n = (length / 60.0) as usize;
        if (0..=n).all(|k| l.fair(a.lerp(b, k as f32 / n as f32), 30.0, 0.05)) {
            l.plan.ribbons.push(Ribbon {
                points: vec![a, b],
                half: s.half * 0.85,
                paved: s.half * 0.85,
                look: Look::Concrete,
                surface: Surface::Coral,
                lift: RUNWAY_LIFT - 0.004,
                across: 4,
            });
            l.plan.clearing.push(Capsule {
                a,
                b,
                radius: s.half + 120.0,
                soft: false,
            });
        }
    }
    // The hangars on the side away from the dispersal, a control building by the runway's middle.
    let side = -s.side;
    let toward = -side * s.right();
    let along = l.rng.signed() * 0.15 * s.length;
    let hall_across = side * (s.half + l.rng.range(170.0, 230.0));
    let count = 3 + l.rng.pick(3);
    let pitch = l.rng.range(68.0, 80.0);
    let mut row = Vec::new();
    for k in 0..count {
        let u = along + (k as f32 - 0.5 * (count - 1) as f32) * pitch;
        let at = s.at(u, hall_across) + Vec2::new(l.rng.signed(), l.rng.signed()) * 2.0;
        if l.fair(at, 30.0, 0.05) && l.off_pieces(at) > 30.0 && l.off_strips(at) > 30.0 {
            l.plan.huts.push(Hut {
                centre: at,
                axis: Vec2::new(toward.y, -toward.x),
                length: l.rng.range(55.0, 65.0),
                half: l.rng.range(18.0, 22.0),
                height: l.rng.range(13.0, 16.0),
                kind: HutKind::Hall,
                colour: HALL_ROOF[l.rng.pick(HALL_ROOF.len())],
                ends: HALL_WALLS[l.rng.pick(HALL_WALLS.len())],
            });
            row.push(at);
        }
    }
    if let (Some(first), Some(last)) = (row.first().copied(), row.last().copied()) {
        // Their apron, to the runway.
        let mid = (first + last) * 0.5 + toward * 55.0;
        let half_len = 0.5 * first.distance(last) + 40.0;
        let ring = rough(&mut l.rng, toward, 32.0, half_len, 6.0, 0.015, 24);
        l.plan.pads.push(Pad {
            centre: mid,
            ring,
            look: Look::ConcretePad,
            lift: LOT_LIFT,
        });
        let link_from = mid + toward * 30.0;
        let link_to = edge(&s, (mid - s.centre).dot(s.dir), side);
        l.plan.ribbons.push(Ribbon {
            points: vec![link_from, link_to],
            half: 10.0,
            paved: 10.0,
            look: Look::Concrete,
            surface: Surface::Coral,
            lift: super::strip::TAXIWAY_LIFT,
            across: 2,
        });
        l.plan.clearing.push(Capsule {
            a: first,
            b: last,
            radius: 70.0,
            soft: false,
        });
        for _ in 0..3 {
            let p = mid
                + perp(toward) * l.rng.signed() * half_len * 0.8
                + toward * l.rng.signed() * 20.0;
            if l.off_pieces(p) > 4.0 {
                l.plan.vehicles.push(Vehicle {
                    centre: p,
                    axis: rotate(toward, l.rng.range(0.0, 360.0)),
                    kind: if l.rng.chance(0.4) {
                        VehicleKind::Bowser
                    } else {
                        VehicleKind::Truck
                    },
                    colour: GREY[l.rng.pick(GREY.len())],
                });
            }
        }
    }
    // The control building by the runway past the hangars' row, either end.
    let past = 0.5 * count as f32 * pitch + l.rng.range(60.0, 140.0);
    let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    let control = [sign, -sign]
        .into_iter()
        .map(|k| s.at(along + k * past, side * (s.half + 70.0)))
        .find(|p| l.fair(*p, 8.0, 0.08) && l.off_pieces(*p) > 10.0 && l.off_strips(*p) > 10.0);
    if let Some(control) = control {
        l.plan.huts.push(Hut {
            centre: control,
            axis: toward,
            length: 10.0,
            half: 6.0,
            height: 9.0,
            kind: HutKind::Control,
            colour: TILE[0],
            ends: BRICK[l.rng.pick(BRICK.len())],
        });
    }
    let reach = Vec2::new(0.5 * s.length + across - 80.0, across - 60.0);
    // Taxiways off into the country and the woods, with their boxes.
    l.soft = true;
    let mut wanted = vec![(-1.0, s.side, false), (1.0, s.side, false)];
    if l.rng.chance(0.6) {
        wanted.push((if l.rng.chance(0.5) { 1.0 } else { -1.0 }, s.side, true));
    }
    for (sign, side, outward) in wanted {
        let length = l.rng.range(550.0, 900.0);
        for _ in 0..3 {
            let along = sign * (0.5 * s.length - l.rng.range(60.0, 200.0));
            if let Some(line) = winding(l, along, side, outward, length, 9.0)
                && l.road_along(&line, 9.0, Look::Concrete)
            {
                let line = l
                    .plan
                    .ribbons
                    .last()
                    .map(|r| r.points.clone())
                    .unwrap_or(line);
                boxes(l, &line, 9.0, true);
                break;
            }
        }
    }
    l.soft = false;
    let hedges = 5 + l.rng.pick(4);
    painted_hedges(l, reach, hedges);
    l.soft = true;
    let behind = s.at(along, hall_across + side * l.rng.range(180.0, 260.0));
    technical(l, behind, side * s.right());
    let (u, v) = (l.rng.signed() * 400.0, l.rng.range(550.0, 800.0));
    village(l, s.at(along + u, hall_across + side * v), side * s.right());
    let light = 6 + l.rng.pick(3);
    flak(l, reach, light, true);
}

/// Dispersal boxes either side of a taxiway `line`, their mouths on it: earth walls in a U grassed over, the larger for
/// bombers at a major base, some under the trees, a few netted.
fn boxes(l: &mut Layout<'_, '_>, line: &[Vec2], half: f32, major: bool) {
    let along = distances(line);
    let total = *along.last().expect("a line");
    let mut at = l.rng.range(80.0, 120.0);
    let mut side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    while at < total - 12.0 {
        let (q, tangent) = point_at(line, &along, at);
        if l.rng.chance(0.6) {
            side = -side;
        }
        let out = perp(tangent) * side;
        let bomber = major && l.rng.chance(0.5);
        let inner = if bomber {
            l.rng.range(13.0, 16.0)
        } else {
            l.rng.range(8.0, 9.5)
        };
        let berm = l.rng.range(4.0, 5.0);
        let arms = [l.rng.range(0.5, 0.9), l.rng.range(0.5, 0.9)];
        let reach = inner + berm;
        let mouth = arms[0].max(arms[1]) * inner;
        let centre = q + out * (half + 3.0 + mouth.max(inner * 0.6));
        let facing = rotate(-out, l.rng.signed() * 10.0);
        let wall = reach + berm + 2.0;
        let fits = l.fair(centre, reach, 0.12)
            && l.plan.ribbons.iter().all(|r| {
                r.points == line
                    || r.look == Look::Hedge
                    || line_distance(centre, &r.points) > r.half + wall
            })
            && l.off_pieces(centre) > wall + 3.0;
        if !fits {
            at += 14.0;
            continue;
        }
        l.plan.revetments.push(Revetment {
            centre,
            facing,
            inner,
            berm,
            height: l.rng.range(2.6, 3.4) + if bomber { 0.8 } else { 0.0 },
            arms,
            colour: SOIL[l.rng.pick(SOIL.len())],
            seed: l.rng.next_u64() as u32,
            horseshoe: false,
        });
        l.plan.stands += 1;
        let under = l.rng.chance(0.35);
        let radius = if under {
            inner + 0.5
        } else {
            reach + berm + 1.0
        };
        l.footprint(centre, centre + facing * mouth, radius);
        if l.rng.chance(l.fill) {
            l.park(centre, facing, inner - 0.5, 0.0);
        }
        if l.rng.chance(0.15) {
            let colour = NETTING[l.rng.pick(NETTING.len())];
            let seed = l.rng.next_u64() as u32;
            l.plan.nets.push(Net {
                centre,
                axis: facing,
                half: Vec2::new(inner, inner * 0.9),
                height: if bomber { 6.0 } else { 4.5 },
                colour,
                fronds: false,
                seed,
            });
        }
        at += 2.0 * (reach + berm) + l.rng.range(20.0, 60.0);
    }
}

/// The technical buildings about `centre`, `away` from the field: brick under red tiles, along a lane at angles of
/// their own.
fn technical(l: &mut Layout<'_, '_>, centre: Vec2, away: Vec2) {
    if !l.dry(centre) {
        return;
    }
    let along = perp(away);
    let lane = [
        centre - along * 160.0,
        centre + away * 10.0,
        centre + along * 160.0,
    ];
    if !l.road_along(&lane, 3.5, Look::Tarmac) {
        return;
    }
    let mut u = -140.0;
    while u < 140.0 {
        let side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let at = centre + along * u + away * (side * l.rng.range(16.0, 30.0));
        let axis = rotate(along, l.rng.signed() * 10.0);
        let length = l.rng.range(18.0, 40.0);
        let half = l.rng.range(5.0, 8.0);
        if l.rng.chance(0.85) && house(l, at, axis, length, half, HutKind::Brick) {
            u += 0.5 * length + l.rng.range(14.0, 26.0);
        } else {
            u += 10.0;
        }
    }
    l.join(centre, 60.0, 3.5);
}

/// A building of `kind` (brick, a farmhouse) at `at` along `axis`, if it fits: brick or render, red tiles.
fn house(
    l: &mut Layout<'_, '_>,
    at: Vec2,
    axis: Vec2,
    length: f32,
    half: f32,
    kind: HutKind,
) -> bool {
    let ends_at = [at - axis * (0.5 * length), at, at + axis * (0.5 * length)];
    if !l.fair(at, 0.5 * length, 0.1)
        || !ends_at
            .iter()
            .all(|p| l.dry(*p) && l.off_strips(*p) > half + 2.0 && l.off_pieces(*p) > half + 3.0)
    {
        return false;
    }
    let walls = if kind == HutKind::Farmhouse && l.rng.chance(0.6) {
        RENDER[l.rng.pick(RENDER.len())]
    } else {
        BRICK[l.rng.pick(BRICK.len())]
    };
    l.plan.huts.push(Hut {
        centre: at,
        axis,
        length,
        half,
        height: 8.0,
        kind,
        colour: TILE[l.rng.pick(TILE.len())],
        ends: walls,
    });
    true
}

/// The barracks as a village: farmhouses along a lane that bends through it, each at its own angle, a gap here and
/// there, a few set back.
fn village(l: &mut Layout<'_, '_>, centre: Vec2, away: Vec2) {
    let mut start = None;
    for k in 0..8 {
        let p = centre + Vec2::from_angle(k as f32 * TAU / 8.0) * (k as f32 * 70.0);
        if l.fair(p, 60.0, 0.08) && l.off_strips(p) > 60.0 && l.off_pieces(p) > 60.0 {
            start = Some(p);
            break;
        }
    }
    let Some(centre) = start else { return };
    let along = rotate(perp(away), l.rng.signed() * 30.0);
    let bend = perp(along) * l.rng.signed() * 40.0;
    let lane = [
        centre - along * 300.0,
        centre + bend,
        centre + along * 300.0,
    ];
    if !l.road_along(&lane, 3.0, Look::Tarmac) {
        return;
    }
    let line = l
        .plan
        .ribbons
        .last()
        .map(|r| r.points.clone())
        .unwrap_or_default();
    let distance = distances(&line);
    let total = *distance.last().unwrap_or(&0.0);
    let mut at = l.rng.range(10.0, 25.0);
    while at < total - 10.0 {
        let (q, tangent) = point_at(&line, &distance, at);
        let side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let length = l.rng.range(16.0, 28.0);
        let half = l.rng.range(5.0, 7.0);
        let end_on = l.rng.chance(0.4);
        let axis = rotate(
            if end_on { perp(tangent) } else { tangent },
            l.rng.signed() * 12.0,
        );
        let breadth = if end_on { 0.5 * length } else { half };
        let at_point = q + perp(tangent) * side * (3.0 + breadth + l.rng.range(4.0, 20.0));
        if l.rng.chance(0.9) && house(l, at_point, axis, length, half, HutKind::Farmhouse) {
            at += breadth + l.rng.range(8.0, 22.0);
        } else {
            at += 10.0;
        }
    }
    l.join(centre, 200.0, 3.0);
}

/// 2 cm Flak in earth rings about the field (`reach` its half extents) and, at a major base, a battery of four 8.8 cm
/// guns off to one side.
fn flak(l: &mut Layout<'_, '_>, reach: Vec2, light: usize, heavy: bool) {
    let s = l.site;
    let mut laid = 0;
    for k in 0..light * 3 {
        if laid >= light {
            break;
        }
        let t = (k as f32 + l.rng.unit()) * TAU / (light * 3) as f32 * 3.0;
        let p = s.at(t.cos() * (reach.x + 40.0), t.sin() * (reach.y + 40.0));
        if l.fair(p, 5.0, 0.15) && l.off_strips(p) > 8.0 && l.off_pieces(p) > 10.0 {
            let out = (p - s.centre).normalize_or(s.dir);
            l.plan.pits.push(Pit {
                centre: p,
                radius: 2.6,
                opening: rotate(-out, l.rng.signed() * 60.0),
                gun: Gun::Flak20,
                aim: rotate(out, l.rng.signed() * 70.0),
            });
            laid += 1;
        }
    }
    if !heavy {
        return;
    }
    for _ in 0..8 {
        let dir = Vec2::from_angle(l.rng.range(0.0, TAU));
        let at = s.centre + dir * l.rng.range(reach.x + 250.0, reach.x + 600.0);
        if !l.fair(at, 30.0, 0.1) || l.off_strips(at) < 40.0 || l.off_pieces(at) < 40.0 {
            continue;
        }
        let aim = rotate(dir, l.rng.signed() * 90.0);
        let turn = l.rng.range(0.0, 90.0);
        for k in 0..4 {
            let d = rotate(s.dir, turn + 90.0 * k as f32);
            let p = at + d * l.rng.range(20.0, 26.0);
            l.plan.pits.push(Pit {
                centre: p,
                radius: 4.0,
                opening: -d,
                gun: Gun::Flak88,
                aim,
            });
            l.footprint(at, p, 3.0);
        }
        l.footprint(at, at, 8.0);
        l.join(at, 30.0, 2.5);
        return;
    }
}

/// The Feldflugplatz.
fn feldflugplatz(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let [e0, e1] = s.ends();
    let across = l.rng.range(160.0, 220.0);
    field(l, across);
    l.plan.ribbons.push(Ribbon {
        points: vec![e0, e1],
        half: s.half,
        paved: s.half,
        look: Look::Mown,
        surface: Surface::Coral,
        lift: RUNWAY_LIFT,
        across: 4,
    });
    let reach = Vec2::new(0.5 * s.length + across - 60.0, across - 30.0);
    // Boxes at the field's edge on both sides, tracks out to them across the grass.
    l.soft = true;
    for side in [s.side, -s.side] {
        let edge_line: Vec<Vec2> = (0..=8)
            .map(|k| {
                s.at(
                    (k as f32 / 8.0 - 0.5) * 0.85 * s.length,
                    side * (across - 10.0),
                )
            })
            .collect();
        if edge_line.iter().all(|p| l.dry(*p)) {
            boxes_at_edge(l, &edge_line, side);
        }
    }
    // A few wooden huts and tents at one end, by the trees.
    let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    let camp = s.at(sign * (0.5 * s.length + 40.0), -s.side * (across - 40.0));
    for k in 0..4 + l.rng.pick(4) {
        let at = camp + rotate(s.dir, l.rng.range(0.0, 360.0)) * l.rng.range(10.0, 50.0);
        let axis = rotate(s.dir, l.rng.signed() * 40.0);
        if !l.fair(at, 6.0, 0.15) || l.off_pieces(at) < 6.0 || l.off_strips(at) < 6.0 {
            continue;
        }
        if k % 3 == 2 {
            l.plan.tents.push(Tent {
                centre: at,
                axis,
                half: 2.5,
                wall: 1.4,
                peak: 3.4,
                colour: CANVAS[l.rng.pick(CANVAS.len())],
                rolled: false,
            });
        } else {
            l.plan.huts.push(Hut {
                centre: at,
                axis,
                length: l.rng.range(8.0, 14.0),
                half: 2.8,
                height: 3.6,
                kind: HutKind::Barracks,
                colour: PLANKS[1],
                ends: PLANKS[0],
            });
        }
    }
    for _ in 0..2 + l.rng.pick(3) {
        let p = camp + rotate(s.dir, l.rng.range(0.0, 360.0)) * l.rng.range(20.0, 60.0);
        if l.fair(p, 3.0, 0.15) && l.off_pieces(p) > 4.0 && l.off_strips(p) > 4.0 {
            l.plan.drums.push(Drums {
                centre: p,
                axis: s.dir,
                along: 3 + l.rng.pick(4) as u8,
                across: 2 + l.rng.pick(2) as u8,
                tiers: 1,
                seed: l.rng.pick(256) as u8,
            });
        }
    }
    l.soft = false;
    let hedges = 2 + l.rng.pick(3);
    painted_hedges(l, reach, hedges);
    l.soft = true;
    let light = 3 + l.rng.pick(2);
    flak(l, reach, light, false);
}

/// Small boxes along the field's edge (`edge_line`, on `side`), under the trees as often as not, their ways out across
/// the grass trodden.
fn boxes_at_edge(l: &mut Layout<'_, '_>, edge_line: &[Vec2], side: f32) {
    let s = l.site;
    let along = distances(edge_line);
    let total = *along.last().expect("a line");
    let mut at = l.rng.range(20.0, 60.0);
    while at < total {
        let (q, _) = point_at(edge_line, &along, at);
        at += l.rng.range(70.0, 130.0);
        let out = s.right() * side;
        let inner = l.rng.range(8.0, 9.0);
        let berm = l.rng.range(2.8, 3.6);
        let centre = q + out * (inner + 6.0);
        let reach = inner + berm;
        if !l.fair(centre, reach, 0.12)
            || l.off_pieces(centre) < 2.0 * reach + 4.0
            || l.off_strips(centre) < reach
        {
            continue;
        }
        l.plan.revetments.push(Revetment {
            centre,
            facing: rotate(-out, l.rng.signed() * 15.0),
            inner,
            berm,
            height: l.rng.range(2.4, 3.0),
            arms: [l.rng.range(0.4, 0.8), l.rng.range(0.4, 0.8)],
            colour: SOIL[l.rng.pick(SOIL.len())],
            seed: l.rng.next_u64() as u32,
            horseshoe: false,
        });
        l.plan.stands += 1;
        let under = l.rng.chance(0.55);
        l.footprint(
            centre,
            centre,
            if under { inner + 0.5 } else { reach + berm },
        );
        // Its way out across the grass trodden, not paved: muddy grass (the land's), no strip laid.
        let to = q - out * l.rng.range(30.0, 60.0);
        l.footprint(centre - out * inner, q, 3.0);
        l.footprint(q, to, 3.0);
        if l.rng.chance(l.fill) {
            l.park(centre, -out, inner - 0.5, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::raf::tests::{brook, farmland};
    use super::super::site::fit;
    use super::super::{Template, lay_out};
    use super::*;

    fn plan(size: SizeClass, seed: u32) -> Plan {
        let input = SiteInput {
            seed,
            size,
            template: Template::LuftwaffeField,
            gravel: false,
            heading_hint: None,
            ground: &farmland,
            water: &brook,
            others: &[],
            runway: None,
        };
        let site = fit(&input).expect("a site");
        lay_out(site, &input)
    }

    #[test]
    fn a_luftwaffe_field_is_laid_out_alike_every_time_and_on_dry_ground() {
        for size in [SizeClass::BomberField, SizeClass::FighterStrip] {
            let a = plan(size, 11);
            assert_eq!(a, plan(size, 11), "laid out alike");
            assert!(a.temperate);
            assert!(
                a.ribbons.iter().any(|r| r.look == Look::Hedge),
                "{size:?}: painted hedges"
            );
            assert!(
                a.revetments.len() >= 4,
                "{size:?}: {} boxes",
                a.revetments.len()
            );
            assert!(a.revetments.iter().all(|r| !r.horseshoe));
            assert!(a.pits.iter().any(|p| p.gun == Gun::Flak20));
            assert!(a.parked.is_empty(), "no Luftwaffe type in the client");
            if size == SizeClass::BomberField {
                assert!(
                    a.ribbons
                        .iter()
                        .any(|r| r.look == Look::Concrete && r.lift >= RUNWAY_LIFT - 0.01)
                );
                assert!(
                    a.huts.iter().filter(|h| h.kind == HutKind::Hall).count() >= 2,
                    "hangars"
                );
                assert!(
                    a.huts
                        .iter()
                        .filter(|h| h.kind == HutKind::Farmhouse)
                        .count()
                        >= 6,
                    "a village"
                );
                assert!(a.huts.iter().any(|h| h.kind == HutKind::Control));
                assert!(a.pits.iter().filter(|p| p.gun == Gun::Flak88).count() == 4);
                assert!(
                    a.revetments.iter().any(|r| r.inner > 12.0),
                    "bombers' boxes"
                );
            } else {
                assert!(a.ribbons.iter().any(|r| r.look == Look::Mown));
                assert!(a.ribbons.iter().all(|r| r.look != Look::Concrete));
                assert!(!a.huts.is_empty() || !a.tents.is_empty());
            }
            for p in a
                .huts
                .iter()
                .map(|h| h.centre)
                .chain(a.revetments.iter().map(|r| r.centre))
                .chain(a.pits.iter().map(|p| p.centre))
                .chain(a.pads.iter().map(|p| p.centre))
            {
                assert!(!brook(p), "{p} wet");
                assert!(a.holds(p), "{p} outside the clearing");
            }
        }
        assert_ne!(
            plan(SizeClass::BomberField, 11).huts,
            plan(SizeClass::BomberField, 12).huts
        );
    }
}
