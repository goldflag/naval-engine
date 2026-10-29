//! The `ijn-solomons-field` template: a Japanese navy airfield of 1943 in the Solomons (Munda, Vila, Ballale, Kahili,
//! Kara, Buka, Bonis) laid out on its site ([`ijn_solomons_field`]), in metres about the base's point. One wide runway
//! of coral and gravel rolled by hand, its edges ragged, no mat; taxiways of packed earth leaving it near its ends and
//! winding off into the jungle, horseshoe revetments of earth dug into the forest along them (many under the trees'
//! crowns, some under netting or palm fronds), most holding a plane (Zeros at a fighter field; Vals, Kates and Zeros at
//! a bomber field) and a few more lined up at the runway's edge by one end; a lookout tower and the headquarters by the
//! runway's middle, a camp of long dark barracks and thatched huts scattered under the trees along a winding road, a
//! wooden hangar at a bomber field, fuel drums dug in here and there, a bowser, a truck or two and hand carts; 25 mm
//! triple mounts in earth pits about the runway and a few heavier guns in their rings beyond. The runway is cleared to
//! bare ground; all else only where it stands (soft: the land's own ground and trees between). Every piece on dry,
//! fair ground and every choice the base's seed's, as the American template's ([`super::strip`]), whose pieces, plan
//! and layout it shares.

use glam::Vec2;

use super::site::{Site, SiteInput};
use super::strip::{
    Capsule, Drums, Gun, Hut, HutKind, Layout, Look, Net, Pit, Plan, ROAD_LIFT, RUNWAY_LIFT,
    Revetment, Ribbon, Surface, Tower, Vehicle, VehicleKind, distances, line_distance, perp,
    point_at, rotate, smooth,
};
use super::{Rng, SizeClass};

/// The types a Japanese field parks (the client's aircraft models) and their wingspans (m): Zeros at a fighter field
/// (the A6M2 mostly, the A6M5 coming that autumn), Vals and Kates at a bomber field with its Zeros.
pub const FIGHTERS: [(&str, f32); 2] = [("a6m2-zero", 12.0), ("a6m5-zero", 11.0)];
pub const BOMBERS: [(&str, f32); 2] = [("d3a1-val", 14.4), ("b5n2-kate", 15.5)];

/// Weathered planks, dark; a roof of boards or tar paper; thatch and woven palm; netting; the trucks' khaki and
/// green; the berms' earth (New Georgia's and Bougainville's, grey-brown).
const WOOD: [[u8; 3]; 4] = [[78, 68, 54], [70, 62, 50], [88, 76, 58], [66, 60, 50]];
const ROOF: [[u8; 3]; 3] = [[56, 52, 46], [62, 58, 50], [50, 48, 44]];
const THATCH: [[u8; 3]; 3] = [[134, 114, 76], [120, 104, 70], [142, 122, 82]];
const WOVEN: [[u8; 3]; 2] = [[126, 108, 80], [112, 98, 74]];
const NETTING: [[u8; 3]; 3] = [[70, 78, 48], [62, 70, 44], [78, 80, 52]];
const KHAKI: [[u8; 3]; 3] = [[104, 94, 64], [92, 86, 60], [64, 70, 48]];
const SOIL: [[u8; 3]; 3] = [[98, 88, 72], [106, 94, 74], [90, 84, 70]];

/// The runway's rough ends past its length (m).
const OVERRUN_M: f32 = 20.0;
/// How far the runway's clearing reaches past its shoulders, and past its ends (m): the jungle close.
const CLEARED_M: f32 = 24.0;
const CLEARED_ENDS_M: f32 = 60.0;
/// A taxiway's step as it winds (m).
const STEP_M: f32 = 26.0;

/// The base laid out on `site`, as `input` (its seed, size, ground and neighbours) allows.
pub fn ijn_solomons_field(site: Site, input: &SiteInput<'_>) -> Plan {
    let fighter = input.size == SizeClass::FighterStrip;
    let mut rng = Rng::new(input.seed, 2);
    let air = if fighter {
        vec![
            (FIGHTERS[0].0, FIGHTERS[0].1, 0.75),
            (FIGHTERS[1].0, FIGHTERS[1].1, 0.25),
        ]
    } else {
        vec![
            (BOMBERS[0].0, BOMBERS[0].1, 0.4),
            (BOMBERS[1].0, BOMBERS[1].1, 0.25),
            (FIGHTERS[0].0, FIGHTERS[0].1, 0.35),
        ]
    };
    let fill = rng.range(0.6, 0.8);
    let mut l = Layout::new(site, input, rng, fighter, air, fill);
    l.road_look = Look::Dirt;
    runway(&mut l);
    // All else stands among the trees.
    l.soft = true;
    let hq = post(&mut l);
    camp(&mut l, hq);
    taxiways(&mut l);
    line_up(&mut l);
    if !fighter {
        hangar(&mut l);
    }
    drums(&mut l);
    guns(&mut l);
    l.finish()
}

fn runway(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let [e0, e1] = s.ends();
    l.plan.ribbons.push(Ribbon {
        points: vec![e0 - s.dir * OVERRUN_M, e1 + s.dir * OVERRUN_M],
        half: s.half + s.shoulder,
        paved: s.half,
        look: Look::Field,
        surface: Surface::Coral,
        lift: RUNWAY_LIFT,
        across: 4,
    });
    l.plan.clearing.push(Capsule {
        a: e0 - s.dir * (OVERRUN_M + CLEARED_ENDS_M),
        b: e1 + s.dir * (OVERRUN_M + CLEARED_ENDS_M),
        radius: s.half + s.shoulder + CLEARED_M,
        soft: false,
    });
}

/// The runway's edge on `side` (+1 its right) at `along` (m from its middle).
pub(super) fn edge(s: &Site, along: f32, side: f32) -> Vec2 {
    s.at(along, side * (s.half + s.shoulder - 2.0))
}

/// The lookout tower by the runway's middle on the camp's side (away from the dispersal, else beside it), the
/// headquarters behind it with an office hut, a road to the runway. Where the camp's road leaves from, and the side.
fn post(l: &mut Layout<'_, '_>) -> Option<(Vec2, f32)> {
    let s = l.site;
    for k in 0..16 {
        let side = if k < 8 { -s.side } else { s.side };
        let away = s.right() * side;
        let along = l.rng.signed() * 0.35 * s.length;
        let tower = s.at(
            along,
            side * (s.half + s.shoulder + l.rng.range(26.0, 34.0)),
        );
        let hq = tower
            + rotate(away, l.rng.signed() * 25.0) * l.rng.range(30.0, 42.0)
            + s.dir * l.rng.signed() * 12.0;
        if !l.fair(tower, 4.0, 0.15) || !l.fair(hq, 14.0, 0.12) || l.off_pieces(hq) < 20.0 {
            continue;
        }
        l.plan.towers.push(Tower {
            centre: tower,
            axis: rotate(s.dir, l.rng.signed() * 8.0),
            height: l.rng.range(12.0, 15.0),
            lookout: true,
        });
        let axis = rotate(s.dir, l.rng.signed() * 14.0);
        l.plan.huts.push(Hut {
            centre: hq,
            axis,
            length: l.rng.range(22.0, 28.0),
            half: 4.6,
            height: 4.6,
            kind: HutKind::Barracks,
            colour: ROOF[l.rng.pick(ROOF.len())],
            ends: WOOD[l.rng.pick(WOOD.len())],
        });
        let office = hq
            + rotate(perp(axis), l.rng.signed() * 20.0)
                * 14.0
                * if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        if l.fair(office, 5.0, 0.15) && l.off_pieces(office) > 3.0 {
            l.plan.huts.push(Hut {
                centre: office,
                axis: rotate(axis, 90.0 + l.rng.signed() * 15.0),
                length: l.rng.range(7.0, 9.0),
                half: 2.6,
                height: 4.6,
                kind: HutKind::Thatch,
                colour: THATCH[l.rng.pick(THATCH.len())],
                ends: WOVEN[l.rng.pick(WOVEN.len())],
            });
        }
        // Past the tower's foot to the runway.
        let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let to = edge(&s, along + sign * l.rng.range(18.0, 35.0), side);
        l.road(hq - away * 9.0 + s.dir * (sign * 4.0), to, 2.6);
        return Some((hq, side));
    }
    None
}

/// The taxiways: two leaving the runway near its ends on the dispersal's side (one runs on past its end), a third at
/// a bomber field from the other side; each with its revetments.
fn taxiways(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let half = if l.fighter { 5.5 } else { 6.5 };
    let mut wanted = vec![
        (-1.0, s.side, l.rng.chance(0.4)),
        (1.0, s.side, l.rng.chance(0.4)),
    ];
    if !l.fighter || l.rng.chance(0.4) {
        let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        wanted.push((sign, -s.side, true));
    }
    for (sign, side, outward) in wanted {
        let length = if l.fighter {
            l.rng.range(420.0, 650.0)
        } else {
            l.rng.range(520.0, 800.0)
        };
        for _ in 0..3 {
            let along = sign * (0.5 * s.length - l.rng.range(40.0, 160.0));
            if let Some(line) = winding(l, along, side, outward, length, half)
                && l.road_along(&line, half, Look::Dirt)
            {
                let line = l
                    .plan
                    .ribbons
                    .last()
                    .map(|r| r.points.clone())
                    .unwrap_or(line);
                revetments(l, &line, half);
                break;
            }
        }
    }
}

/// A taxiway's line leaving the runway's edge at `along` on `side`, off at an angle and then winding on some way off
/// the runway (toward its middle, or with `outward` past its end) into the jungle, `length` long at most: kept on dry
/// ground and clear of the strips laid. None if it would be short.
pub(super) fn winding(
    l: &mut Layout<'_, '_>,
    along: f32,
    side: f32,
    outward: bool,
    length: f32,
    half: f32,
) -> Option<Vec<Vec2>> {
    let s = l.site;
    let away = s.right() * side;
    let onward = s.dir
        * if outward {
            along.signum()
        } else {
            -along.signum()
        };
    let offset = if outward {
        l.rng.range(60.0, 130.0)
    } else {
        l.rng.range(95.0, 190.0)
    };
    let start = edge(&s, along, side);
    let mut dir = (away + onward * l.rng.range(0.3, 0.8)).normalize();
    let mut points = vec![start];
    let mut p = start;
    let mut run = 0.0;
    let wander = l.rng.range(10.0, 22.0);
    while run < length {
        let across = (p - s.centre).dot(s.right()) * side;
        // Toward its offset off the runway, then on.
        let want = (onward + away * ((offset - across) / 50.0).clamp(-1.2, 1.2)).normalize();
        let steer = dir.perp_dot(want).clamp(-1.0, 1.0).asin().to_degrees() * 0.4;
        let turn = (steer + l.rng.signed() * wander).clamp(-32.0, 32.0);
        let mut next = None;
        for extra in [0.0f32, 30.0, -30.0, 60.0, -60.0] {
            let d = rotate(dir, turn + extra);
            let q = p + d * STEP_M;
            let near_start = q.distance(start) < 60.0;
            // Off the runways (any strip laid at a runway's height) but where it leaves one; off the rest by 25 m.
            let clear = l.plan.ribbons.iter().all(|r| {
                if r.look == Look::Hedge {
                    return true;
                }
                let runway = r.lift >= RUNWAY_LIFT;
                let room = if runway { 0.0 } else { 25.0 };
                (near_start && runway) || line_distance(q, &r.points) > r.half + half + room
            });
            if l.fair(q, half + 4.0, 0.12) && clear && l.off_pieces(q) > half + 8.0 {
                next = Some((q, d));
                break;
            }
        }
        let Some((q, d)) = next else { break };
        points.push(q);
        p = q;
        dir = d;
        run += STEP_M;
    }
    (run >= 0.45 * length && points.len() >= 6).then(|| smooth(&points, 2))
}

/// Horseshoe revetments either side of a taxiway `line` (`half` its half width), their mouths on it: dug into the
/// forest, many with the trees standing over them on their banks, some netted or strewn with fronds; most holding a
/// plane, its nose out.
fn revetments(l: &mut Layout<'_, '_>, line: &[Vec2], half: f32) {
    let along = distances(line);
    let total = *along.last().expect("a line");
    let widest = l.air.iter().map(|a| a.1).fold(0.0, f32::max);
    let mut at = l.rng.range(70.0, 100.0);
    let mut side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    while at < total - 12.0 {
        let (q, tangent) = point_at(line, &along, at);
        if l.rng.chance(0.7) {
            side = -side;
        }
        let out = perp(tangent) * side;
        let inner = 0.5 * widest + l.rng.range(2.0, 3.2);
        let berm = l.rng.range(3.2, 4.3);
        let arms = [l.rng.range(0.9, 1.3), l.rng.range(0.9, 1.3)];
        let reach = inner + berm;
        let mouth = arms[0].max(arms[1]) * inner;
        let centre = q + out * (half + 2.5 + mouth) + tangent * l.rng.signed() * 4.0;
        let facing = rotate(-out, l.rng.signed() * 15.0);
        let wall = reach + berm + 2.0;
        let fits = l.fair(centre, reach, 0.18)
            && l.plan.ribbons.iter().all(|r| {
                if r.points == line {
                    line_distance(centre, &r.points) >= half + 2.0 + mouth - 1.0
                } else {
                    line_distance(centre, &r.points) > r.half + wall
                }
            })
            && l.off_pieces(centre) > wall + 3.0;
        if !fits {
            at += 14.0;
            continue;
        }
        let soil = SOIL[l.rng.pick(SOIL.len())];
        l.plan.revetments.push(Revetment {
            centre,
            facing,
            inner,
            berm,
            height: l.rng.range(2.6, 3.6),
            arms,
            colour: soil,
            seed: l.rng.next_u64() as u32,
            horseshoe: true,
        });
        l.plan.stands += 1;
        // Under the trees (their crowns over it, the banks heaped about their trunks), or cut clear to its banks' feet.
        let under = l.rng.chance(0.45);
        let radius = if under {
            inner + 0.5
        } else {
            reach + berm + 1.0
        };
        l.footprint(centre, centre + facing * mouth, radius);
        let parked = l.rng.chance(l.fill);
        let before = l.plan.parked.len();
        if parked {
            l.park(centre - facing * (0.1 * inner), facing, inner - 0.5, 0.0);
        }
        let plane = l.plan.parked.get(before).copied();
        let cover = l.rng.unit();
        let colour = NETTING[l.rng.pick(NETTING.len())];
        let seed = l.rng.next_u64() as u32;
        if cover < 0.22 {
            // Netting on poles over the whole of its inside.
            l.plan.nets.push(Net {
                centre: centre + facing * (0.2 * inner),
                axis: facing,
                half: Vec2::new(inner * 1.05, inner * 0.95),
                height: if l.fighter { 4.0 } else { 4.6 } + l.rng.signed() * 0.3,
                colour,
                fronds: false,
                seed,
            });
        } else if let Some(p) = plane.filter(|_| cover < 0.4) {
            // Palm fronds laid over the plane.
            let span = l.air.iter().find(|a| a.0 == p.model).map_or(12.0, |a| a.1);
            l.plan.nets.push(Net {
                centre: p.centre,
                axis: facing,
                half: Vec2::new(0.42 * span, 0.5 * span),
                height: 1.9,
                colour,
                fronds: true,
                seed,
            });
        }
        at += 2.0 * (reach + berm) + l.rng.range(14.0, 45.0);
    }
}

/// A few planes lined up at the runway's edge by one end, the bowser by them.
fn line_up(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    let side = if l.rng.chance(0.6) { s.side } else { -s.side };
    let widest = l.air.iter().map(|a| a.1).fold(0.0, f32::max);
    let across = side * (s.half + s.shoulder + 3.0 + 0.5 * widest);
    let count = 3 + l.rng.pick(4);
    let mut along = sign * (0.5 * s.length - l.rng.range(50.0, 110.0));
    let toward = rotate(-s.right() * side, sign * side * l.rng.range(0.0, 40.0));
    let mut last = None;
    for _ in 0..count {
        let at = s.at(along, across + l.rng.signed() * 2.0);
        if l.fair(at, 0.5 * widest, 0.08) && l.off_pieces(at) > 1.0 && l.rng.chance(0.85) {
            l.park(at, toward, 0.5 * widest + 2.0, 0.0);
            last = Some(at);
        }
        along -= sign * (widest + l.rng.range(3.0, 9.0));
    }
    if let Some(at) = last {
        let p = at - s.dir * sign * (0.5 * widest + 8.0) + s.right() * side * 4.0;
        if l.fair(p, 4.0, 0.12) && l.off_pieces(p) > 3.0 {
            l.plan.vehicles.push(Vehicle {
                centre: p,
                axis: rotate(s.dir, l.rng.signed() * 40.0),
                kind: VehicleKind::Bowser,
                colour: KHAKI[l.rng.pick(KHAKI.len())],
            });
        }
    }
}

/// A wooden hangar at a bomber field, near an end, open toward the runway, its apron of earth.
fn hangar(l: &mut Layout<'_, '_>) {
    let s = l.site;
    for _ in 0..6 {
        let side = if l.rng.chance(0.5) { s.side } else { -s.side };
        let along = l.rng.signed() * 0.4 * s.length;
        let depth = l.rng.range(20.0, 26.0);
        let half = l.rng.range(14.0, 17.0);
        let mouth = s.half + s.shoulder + l.rng.range(22.0, 34.0);
        let centre = s.at(along, side * (mouth + 0.5 * depth));
        let wall = half.max(depth) + 4.0;
        if !l.fair(centre, wall, 0.08)
            || l.off_strips(centre) < wall
            || l.off_pieces(centre) < wall + 6.0
        {
            continue;
        }
        let toward = -s.right() * side;
        l.plan.huts.push(Hut {
            centre,
            axis: toward,
            length: depth,
            half,
            height: l.rng.range(8.5, 10.0),
            kind: HutKind::Hangar,
            colour: ROOF[l.rng.pick(ROOF.len())],
            ends: WOOD[l.rng.pick(WOOD.len())],
        });
        let (a, b) = (centre + toward * (0.5 * depth - 2.0), edge(&s, along, side));
        l.ribbon(vec![a, b], half - 2.0, Look::Dirt, ROAD_LIFT);
        l.footprint(a, b, half);
        return;
    }
}

/// The camp: a road winding from the headquarters (or the runway's edge) away from the runway into the trees,
/// barracks and thatched huts set off it on both sides at their own angles, a gap here and there; a truck on it.
fn camp(l: &mut Layout<'_, '_>, hq: Option<(Vec2, f32)>) {
    let s = l.site;
    let length = if l.fighter {
        l.rng.range(380.0, 520.0)
    } else {
        l.rng.range(480.0, 680.0)
    };
    // From the headquarters, straight out from its long side; else from the runway's edge on either side.
    let edge_m = s.half + s.shoulder + 30.0;
    let mut starts: Vec<(Vec2, f32, f32)> = Vec::new();
    if let Some((hq, side)) = hq {
        starts.extend([(hq, side, 10.0); 4]);
    }
    for side in [-s.side, s.side] {
        for _ in 0..3 {
            let along = l.rng.signed() * 0.4 * s.length;
            starts.push((s.at(along, side * edge_m), side, 0.0));
        }
    }
    let mut line = Vec::new();
    for (start, side, out) in starts {
        let away = side * s.right();
        let mut dir = rotate(away, l.rng.signed() * 40.0);
        let mut p = start + away * out;
        if l.off_pieces(p) < 4.5 || !l.dry(p) {
            continue;
        }
        let mut points = vec![p];
        let mut run = 0.0;
        while run < length {
            let turn = l.rng.signed() * 24.0;
            let mut next = None;
            for extra in [0.0f32, 35.0, -35.0, 70.0, -70.0, 105.0, -105.0] {
                let d = rotate(dir, turn + extra);
                // Away from the runway still, or along it.
                if d.dot(away) < -0.3 {
                    continue;
                }
                let q = p + d * 24.0;
                if l.fair(q, 5.0, 0.18) && l.off_strips(q) > 12.0 && l.off_pieces(q) > 6.0 {
                    next = Some((q, d));
                    break;
                }
            }
            let Some((q, d)) = next else { break };
            points.push(q);
            p = q;
            dir = d;
            run += 24.0;
        }
        if run >= 0.4 * length && l.road_along(&smooth(&points, 2), 2.6, Look::Dirt) {
            line = l
                .plan
                .ribbons
                .last()
                .map(|r| r.points.clone())
                .unwrap_or_default();
            break;
        }
    }
    if line.len() < 2 {
        return;
    }
    let along = distances(&line);
    let total = *along.last().unwrap_or(&0.0);
    let mut at = l.rng.range(20.0, 35.0);
    while at < total - 8.0 {
        let (q, tangent) = point_at(&line, &along, at);
        let side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let roll = l.rng.unit();
        let (kind, length, half, height, colour, ends) = if roll < 0.55 {
            (
                HutKind::Barracks,
                l.rng.range(18.0, 32.0),
                l.rng.range(3.0, 3.8),
                4.0,
                ROOF[l.rng.pick(ROOF.len())],
                WOOD[l.rng.pick(WOOD.len())],
            )
        } else if roll < 0.85 {
            (
                HutKind::Thatch,
                l.rng.range(6.0, 9.5),
                l.rng.range(2.2, 2.9),
                4.4,
                THATCH[l.rng.pick(THATCH.len())],
                WOVEN[l.rng.pick(WOVEN.len())],
            )
        } else {
            (
                HutKind::Barracks,
                l.rng.range(8.0, 11.0),
                2.6,
                3.6,
                ROOF[l.rng.pick(ROOF.len())],
                WOOD[l.rng.pick(WOOD.len())],
            )
        };
        // Along the road more or less, or end on to it.
        let turn = if l.rng.chance(0.3) {
            90.0 + l.rng.signed() * 20.0
        } else {
            l.rng.signed() * 28.0
        };
        let axis = rotate(tangent, turn);
        let out = perp(tangent) * side;
        // Set back from the road by its own breadth across it and some.
        let breadth = (axis.dot(out).abs() * 0.5 * length).max(half);
        let centre = q + out * (2.6 + breadth + l.rng.range(4.0, 16.0));
        let ends_at = [
            centre - axis * (0.5 * length),
            centre,
            centre + axis * (0.5 * length),
        ];
        let fits = l.rng.chance(0.85)
            && l.fair(centre, 0.5 * length, 0.15)
            && ends_at.iter().all(|p| {
                l.dry(*p) && l.off_strips(*p) > half + 2.0 && l.off_pieces(*p) > half + 3.0
            });
        if fits {
            l.plan.huts.push(Hut {
                centre,
                axis,
                length,
                half,
                height,
                kind,
                colour,
                ends,
            });
            // A path to the door now and then.
            if l.rng.chance(0.35) {
                let door = centre - out * (breadth + 0.5);
                l.road_along(&[door, q + out * 2.0], 0.9, Look::Track);
            }
            at += breadth + l.rng.range(12.0, 36.0);
        } else {
            at += 8.0;
        }
    }
    // A truck or two on the road.
    for _ in 0..1 + l.rng.pick(2) {
        let (q, tangent) = point_at(&line, &along, l.rng.range(0.2, 0.9) * total);
        if l.off_pieces(q) > 4.0 {
            let back = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
            l.plan.vehicles.push(Vehicle {
                centre: q,
                axis: tangent * back,
                kind: VehicleKind::Truck,
                colour: KHAKI[l.rng.pick(KHAKI.len())],
            });
        }
    }
}

/// Fuel drums in small lots dug in beside the taxiways and the camp's road, a low bank of earth about each; hand carts
/// by some.
fn drums(l: &mut Layout<'_, '_>) {
    let lines: Vec<Vec<Vec2>> = l
        .plan
        .ribbons
        .iter()
        .filter(|r| r.look == Look::Dirt)
        .map(|r| r.points.clone())
        .collect();
    if lines.is_empty() {
        return;
    }
    let want = if l.fighter { 8 } else { 12 };
    let mut laid = 0;
    for _ in 0..want * 6 {
        if laid >= want {
            break;
        }
        let line = &lines[l.rng.pick(lines.len())];
        if line.len() < 2 {
            continue;
        }
        let along = distances(line);
        let total = *along.last().expect("a line");
        let (q, tangent) = point_at(line, &along, l.rng.unit() * total);
        let side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
        let p = q + perp(tangent) * side * l.rng.range(11.0, 20.0);
        if !l.fair(p, 3.5, 0.2) || l.off_strips(p) < 6.0 || l.off_pieces(p) < 6.0 {
            continue;
        }
        let axis = rotate(tangent, l.rng.signed() * 40.0);
        let (along_n, across_n) = (2 + l.rng.pick(3) as u8, 1 + l.rng.pick(3) as u8);
        l.plan.drums.push(Drums {
            centre: p,
            axis,
            along: along_n,
            across: across_n,
            tiers: 1,
            seed: l.rng.pick(256) as u8,
        });
        let size = 0.5 * f32::from(along_n.max(across_n)) * super::strip::DRUM_PITCH_M;
        l.plan.revetments.push(Revetment {
            centre: p,
            facing: (q - p).normalize_or(axis),
            inner: size + 0.7,
            berm: 0.9,
            height: 0.8,
            arms: [0.7, 0.7],
            colour: SOIL[l.rng.pick(SOIL.len())],
            seed: l.rng.next_u64() as u32,
            horseshoe: true,
        });
        if l.rng.chance(0.35) {
            let c = p + rotate((q - p).normalize_or(axis), l.rng.signed() * 50.0) * (size + 3.5);
            if l.off_pieces(c) > 1.0 {
                l.plan.vehicles.push(Vehicle {
                    centre: c,
                    axis: rotate(axis, l.rng.range(0.0, 360.0)),
                    kind: VehicleKind::Cart,
                    colour: WOOD[l.rng.pick(WOOD.len())],
                });
            }
        }
        laid += 1;
    }
}

/// 25 mm triple mounts in earth pits about the runway and past its ends; a few heavier guns in their rings off to one
/// side, a track to them.
fn guns(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let reach = s.half + s.shoulder + l.rng.range(16.0, 26.0);
    let mut places = Vec::new();
    for along in [
        -0.5 * s.length - 30.0,
        -0.22 * s.length,
        0.1 * s.length,
        0.5 * s.length + 30.0,
    ] {
        for side in [1.0f32, -1.0] {
            places.push((
                along + l.rng.signed() * 40.0,
                side * (reach + l.rng.range(0.0, 14.0)),
            ));
        }
    }
    let mut laid = 0;
    let most = if l.fighter { 6 } else { 8 };
    for (along, across) in places {
        let centre = s.at(along, across);
        if laid >= most || !l.rng.chance(0.75) {
            continue;
        }
        if l.fair(centre, 5.0, 0.15) && l.off_strips(centre) > 5.0 && l.off_pieces(centre) > 6.0 {
            let out = (centre - s.centre).normalize_or(s.dir);
            l.plan.pits.push(Pit {
                centre,
                radius: 2.7,
                opening: rotate(-out, l.rng.signed() * 60.0),
                gun: Gun::Triple,
                aim: rotate(out, l.rng.signed() * 70.0),
            });
            laid += 1;
        }
    }
    // The heavier guns: three or four in a row or a crescent, some 300 m off on the camp's side or past an end.
    let count = 3 + l.rng.pick(2);
    for _ in 0..8 {
        let at = if l.rng.chance(0.5) {
            s.at(
                l.rng.signed() * 0.3 * s.length,
                -s.side * l.rng.range(220.0, 340.0),
            )
        } else {
            let sign = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
            s.at(
                sign * (0.5 * s.length + l.rng.range(180.0, 300.0)),
                l.rng.signed() * 120.0,
            )
        };
        if !l.fair(at, 30.0, 0.12) || l.off_strips(at) < 40.0 || l.off_pieces(at) < 40.0 {
            continue;
        }
        let axis = rotate(s.dir, l.rng.range(0.0, 180.0));
        let aim = rotate(s.right(), l.rng.range(0.0, 360.0));
        for k in 0..count {
            let u = (k as f32 - 0.5 * (count - 1) as f32) * 22.0;
            let bow = perp(axis) * (u * u / 120.0);
            let centre = at + axis * u + bow;
            if l.fair(centre, 5.0, 0.15) {
                l.plan.pits.push(Pit {
                    centre,
                    radius: 3.6,
                    opening: -perp(axis),
                    gun: Gun::Navy,
                    aim,
                });
                l.footprint(at, centre, 3.0);
            }
        }
        l.footprint(at, at, 7.0);
        l.join(at, 20.0, 1.8);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::super::site::{fit, tests::coast, tests::creek};
    use super::super::{Template, lay_out};
    use super::*;

    fn plan(size: SizeClass, seed: u32) -> Plan {
        let input = SiteInput {
            seed,
            size,
            template: Template::IjnSolomonsField,
            gravel: false,
            heading_hint: None,
            ground: &coast,
            water: &creek,
            others: &[],
            runway: None,
        };
        let site = fit(&input).expect("a site");
        lay_out(site, &input)
    }

    #[test]
    fn a_japanese_field_is_laid_out_alike_every_time_and_on_dry_ground() {
        for size in [SizeClass::FighterStrip, SizeClass::BomberField] {
            let a = plan(size, 41);
            assert_eq!(a, plan(size, 41), "laid out alike");
            let site = a.site.expect("a site");
            // One wide runway of rough coral, no mat, no tents nor Quonsets.
            let runways: Vec<&Ribbon> =
                a.ribbons.iter().filter(|r| r.look == Look::Field).collect();
            assert_eq!(runways.len(), 1);
            assert!(
                (50.0..=62.0).contains(&(2.0 * runways[0].half)),
                "{}",
                runways[0].half
            );
            assert!((1000.0..=1400.0).contains(&site.length), "{}", site.length);
            assert!(a.ribbons.iter().all(|r| r.surface == Surface::Coral));
            assert!(a.tents.is_empty());
            assert!(a.huts.iter().all(|h| matches!(
                h.kind,
                HutKind::Barracks | HutKind::Thatch | HutKind::Hangar
            )));
            assert!(
                a.huts
                    .iter()
                    .filter(|h| h.kind == HutKind::Barracks)
                    .count()
                    >= 3,
                "{size:?}: barracks"
            );
            assert!(
                a.huts.iter().any(|h| h.kind == HutKind::Thatch),
                "{size:?}: thatch"
            );
            assert_eq!(
                a.huts.iter().any(|h| h.kind == HutKind::Hangar),
                size == SizeClass::BomberField,
                "{size:?}: a hangar at a bomber field only"
            );
            assert!(a.towers.iter().all(|t| t.lookout) && !a.towers.is_empty());
            assert!(
                a.pits.iter().any(|p| p.gun == Gun::Triple),
                "{size:?}: 25 mm"
            );
            assert!(
                a.pits
                    .iter()
                    .all(|p| matches!(p.gun, Gun::Triple | Gun::Navy))
            );
            assert!(
                a.vehicles
                    .iter()
                    .any(|v| v.kind == VehicleKind::Bowser || v.kind == VehicleKind::Cart)
            );
            assert!(!a.drums.is_empty());
            // Taxiways winding off into the jungle, with horseshoe revetments along them.
            let taxiways = a
                .ribbons
                .iter()
                .filter(|r| r.look == Look::Dirt && r.half >= 4.0)
                .count();
            assert!(taxiways >= 2, "{size:?}: {taxiways} taxiways");
            let stands = a.revetments.iter().filter(|r| r.inner > 5.0).count();
            assert!(
                stands >= 6 && stands == a.stands,
                "{size:?}: {stands} revetments, {} stands",
                a.stands
            );
            assert!(a.revetments.iter().all(|r| r.horseshoe));
            assert!(!a.nets.is_empty(), "{size:?}: no camouflage");
            // Most revetments hold a plane, of the base's types, none on the runway.
            assert!(
                a.parked.len() >= a.stands / 2,
                "{size:?}: {} planes",
                a.parked.len()
            );
            for p in &a.parked {
                assert!(
                    FIGHTERS
                        .iter()
                        .chain(&BOMBERS)
                        .any(|(id, _)| *id == p.model),
                    "{}",
                    p.model
                );
                let d = p.centre - site.centre;
                let on = d.dot(site.right()).abs() < site.half + site.shoulder
                    && d.dot(site.dir).abs() < 0.5 * site.length + OVERRUN_M;
                assert!(!on, "a plane on the runway at {}", p.centre);
            }
            if size == SizeClass::FighterStrip {
                assert!(a.parked.iter().all(|p| p.model.starts_with("a6m")));
            }
            // Every piece on dry ground, in the clearing; the runway's clearing hard, all else soft.
            let points = a
                .huts
                .iter()
                .map(|h| h.centre)
                .chain(a.revetments.iter().map(|r| r.centre))
                .chain(a.pits.iter().map(|p| p.centre))
                .chain(a.towers.iter().map(|t| t.centre))
                .chain(a.drums.iter().map(|d| d.centre))
                .chain(a.vehicles.iter().map(|v| v.centre))
                .chain(a.parked.iter().map(|p| p.centre));
            for p in points {
                assert!(
                    !creek(p) && coast(p) >= super::super::site::DRY_M,
                    "{p} wet"
                );
                assert!(a.holds(p), "{p} outside the clearing");
            }
            assert_eq!(
                a.clearing.iter().filter(|c| !c.soft).count(),
                1,
                "only the runway cleared bare"
            );
            for r in &a.ribbons {
                for p in &r.points {
                    assert!(!creek(*p), "a strip at {p} in the water");
                }
            }
            // The camp is no grid: its huts at many angles.
            let mut angles: Vec<i32> = a
                .huts
                .iter()
                .map(|h| (h.axis.y.atan2(h.axis.x).to_degrees().rem_euclid(180.0) / 10.0) as i32)
                .collect();
            angles.sort_unstable();
            angles.dedup();
            assert!(angles.len() >= 3, "{size:?}: huts at {angles:?}");
            // A revetment's mouth on its taxiway: the taxiway passes within its arms' reach.
            for r in a.revetments.iter().filter(|r| r.inner > 5.0) {
                let near = a
                    .ribbons
                    .iter()
                    .filter(|t| t.look == Look::Dirt && t.half >= 4.0)
                    .map(|t| line_distance(r.centre, &t.points) - t.half)
                    .fold(f32::MAX, f32::min);
                assert!(
                    near < r.inner * 1.4 + 6.0,
                    "a revetment {near} m off its taxiway"
                );
            }
        }
        assert_ne!(
            plan(SizeClass::BomberField, 41).huts,
            plan(SizeClass::BomberField, 42).huts
        );
    }
}
