//! The `raf-station` template: an RAF station of 1943 laid out on its site ([`raf_station`]), in metres about the
//! base's point, in its farmland. A bomber station is the Class A standard: three tarmac runways in a triangle (the
//! main 2,000 yd, two of 1,400 yd crossing it near its ends and each other at the apex, 50 yd wide), a perimeter track
//! 15 m wide through their ends, some thirty dispersal hardstands off it (round frying pans, and pairs side by side,
//! the spectacles), two T2 hangars on the technical side with their aprons, the watch office between the track and
//! the main runway, a technical site of Nissen huts and brick workshops with its water tower and fuel tanks, three to
//! five accommodation sites of Nissen huts dispersed down lanes among the fields, a bomb dump of earth traverses off in
//! a far corner, Bofors pits and pillboxes about it. A fighter station is grass: one or two landing runs mown paler
//! across a field of grass (gravel at Kaldaðarnes), a tarmac perimeter track round it, E-shaped earth blast pens and
//! blister hangars off the track, a smaller watch office, Nissen huts beyond. The airfield is cleared to mown grass
//! (hard, temperate); all else among the fields and hedges only where it stands. Parks nothing (the client has no RAF
//! bomber or fighter; its Walrus has no wheels).

use std::f32::consts::TAU;

use glam::Vec2;

use super::site::{Site, SiteInput};
use super::strip::{
    Bank, Capsule, Crates, Gun, Hut, HutKind, LOT_LIFT, Layout, Look, PAD_LIFT, Pad, Pillbox, Pit,
    Plan, RUNWAY_LIFT, Ribbon, Surface, TAXIWAY_LIFT, Tank, Vehicle, VehicleKind, distances, perp,
    point_at, rotate, rough,
};
use super::{Rng, SizeClass};

/// Nissen huts' tarred and painted steel; a T2's walls and roof; brick and slate; render; the dump's earth; the RAF's
/// blue-grey vehicles.
const NISSEN: [[u8; 3]; 3] = [[58, 60, 56], [66, 66, 60], [74, 76, 70]];
const T2_WALLS: [[u8; 3]; 2] = [[88, 92, 86], [78, 84, 78]];
const T2_ROOF: [[u8; 3]; 2] = [[70, 74, 70], [62, 66, 62]];
const BRICK: [[u8; 3]; 3] = [[136, 82, 64], [124, 76, 60], [146, 92, 70]];
const SLATE: [[u8; 3]; 2] = [[72, 74, 78], [64, 66, 70]];
const RENDER: [[u8; 3]; 2] = [[204, 198, 184], [192, 188, 176]];
const SOIL: [[u8; 3]; 2] = [[92, 96, 66], [98, 98, 70]];
const RAF_BLUE: [[u8; 3]; 2] = [[86, 92, 98], [78, 84, 90]];

/// The perimeter track's half width (m): 50 ft.
const TRACK_HALF_M: f32 = 7.5;
/// A frying pan's radius (m): 125 ft across.
const PAN_M: f32 = 19.0;

/// The station laid out on `site`, as `input` (its seed, size, ground, neighbours and catalog entry) allows.
pub fn raf_station(site: Site, input: &SiteInput<'_>) -> Plan {
    let fighter = input.size == SizeClass::FighterStrip;
    let mut rng = Rng::new(input.seed, 3);
    // No RAF type in the client: its stands stand empty (`Layout::park` parks none).
    let fill = rng.range(0.2, 0.35);
    let mut l = Layout::new(site, input, rng, fighter, Vec::new(), fill);
    l.road_look = Look::Tarmac;
    if fighter {
        grass_field(&mut l);
    } else {
        class_a(&mut l);
    }
    l.finish()
}

/// A closed line through `points` with its corners cut `n` times.
pub(super) fn smooth_loop(points: &[Vec2], n: usize) -> Vec<Vec2> {
    let mut line = points.to_vec();
    for _ in 0..n {
        let mut next = Vec::with_capacity(line.len() * 2);
        for k in 0..line.len() {
            let (a, b) = (line[k], line[(k + 1) % line.len()]);
            next.push(a.lerp(b, 0.25));
            next.push(a.lerp(b, 0.75));
        }
        line = next;
    }
    line.push(line[0]);
    line
}

/// A strip laid along `line` (no checks: the airfield's own ground), cleared to grass about it.
fn paved(l: &mut Layout<'_, '_>, line: Vec<Vec2>, half: f32, look: Look, lift: f32, clear: f32) {
    for w in line.windows(2) {
        l.plan.clearing.push(Capsule {
            a: w[0],
            b: w[1],
            radius: half + clear,
            soft: false,
        });
    }
    l.plan.ribbons.push(Ribbon {
        points: line,
        half,
        paved: half,
        look,
        surface: Surface::Coral,
        lift,
        across: if lift >= RUNWAY_LIFT { 4 } else { 2 },
    });
}

/// The field's edge pushed out here and there (a few capsules about its rim), so its outline is no capsule's: the
/// fields it took in.
pub(super) fn bulges(l: &mut Layout<'_, '_>, reach: Vec2) {
    let s = l.site;
    for _ in 0..4 + l.rng.pick(3) {
        let t = l.rng.range(0.0, TAU);
        let at = s.at(t.cos() * reach.x * 0.8, t.sin() * reach.y * 0.8);
        let radius = l.rng.range(0.3, 0.55) * reach.y;
        if l.dry(at) {
            l.plan.clearing.push(Capsule {
                a: at,
                b: at + rotate(s.dir, l.rng.range(0.0, 180.0)) * l.rng.range(0.0, 0.6) * reach.y,
                radius,
                soft: false,
            });
        }
    }
}

/// Whether every point of the line from `a` to `b` (every 60 m) is dry and fair enough for a runway.
fn runs_fair(l: &Layout<'_, '_>, a: Vec2, b: Vec2, r: f32) -> bool {
    let n = (a.distance(b) / 60.0).ceil().max(1.0) as usize;
    (0..=n).all(|k| l.fair(a.lerp(b, k as f32 / n as f32), r, 0.05))
}

/// The Class A bomber station.
fn class_a(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let [e0, e1] = s.ends();
    // The triangle: its base on the main runway (1,000 m of it about the middle), its apex on the drier side; the two
    // shorter runways along its sides, each crossing the main runway and the other 140 m past the corners.
    let side_m = 1000.0;
    let mut chosen = None;
    for sd in [s.side, -s.side] {
        let (a, b) = (s.at(-0.5 * side_m, 0.0), s.at(0.5 * side_m, 0.0));
        let c = s.at(0.0, sd * 0.866 * side_m);
        let (d2, d3) = ((c - b).normalize(), (c - a).normalize());
        let r2 = [b - d2 * 140.0, c + d2 * 140.0];
        let r3 = [a - d3 * 140.0, c + d3 * 140.0];
        if runs_fair(l, r2[0], r2[1], 30.0) && runs_fair(l, r3[0], r3[1], 30.0) {
            chosen = Some((sd, c, r2, r3));
            break;
        }
    }
    let half = s.half;
    paved(l, vec![e0, e1], half, Look::Tarmac, RUNWAY_LIFT, 45.0);
    let mut ends = vec![e0, e1];
    let (sd, apex) = match chosen {
        Some((sd, c, r2, r3)) => {
            paved(
                l,
                r2.to_vec(),
                half,
                Look::Tarmac,
                RUNWAY_LIFT - 0.004,
                45.0,
            );
            paved(
                l,
                r3.to_vec(),
                half,
                Look::Tarmac,
                RUNWAY_LIFT - 0.008,
                45.0,
            );
            ends.extend(r2);
            ends.extend(r3);
            (sd, c)
        }
        None => (s.side, s.at(0.0, s.side * 500.0)),
    };
    // The grass inside the triangle and about it, mown.
    let middle = ends.iter().copied().sum::<Vec2>() / ends.len() as f32;
    for e in &ends {
        l.plan.clearing.push(Capsule {
            a: middle,
            b: middle.lerp(*e, 0.7),
            radius: 260.0,
            soft: false,
        });
    }
    // The perimeter track through the runways' ends, round the outside (ordered about the middle), its corners
    // rounded.
    let mut ring = ends.clone();
    ring.sort_by(|p, q| {
        (*p - middle)
            .to_angle()
            .total_cmp(&(*q - middle).to_angle())
    });
    let track = smooth_loop(&ring, 3);
    paved(
        l,
        track.clone(),
        TRACK_HALF_M,
        Look::Tarmac,
        TAXIWAY_LIFT,
        30.0,
    );
    let tech = -sd;
    let outer = (s.half + 98.0) * tech;
    let shift = l.rng.signed() * 0.05 * s.length;
    watch_office(l, s.at(shift, tech * 70.0), tech, false);
    // The technical side: two T2s just off the track, their aprons; the technical site beyond them.
    l.soft = true;
    let along = l.rng.signed() * 0.15 * s.length;
    for k in [-1.0f32, 1.0] {
        let at = s.at(along + k * l.rng.range(70.0, 95.0), outer + tech * 70.0);
        t2(l, at, -tech * s.right());
    }
    technical_site(l, s.at(along, outer + tech * 260.0), tech * s.right());
    // The hardstands round the track, clear of the hangars and the technical site.
    l.soft = false;
    hardstands(l, &track, middle, &ends);
    l.soft = true;
    let sites = 3 + l.rng.pick(3);
    accommodation(l, middle, tech * s.right(), sites);
    bomb_dump(l, apex + (apex - middle).normalize_or(s.right()) * 420.0);
    let (pits, boxes) = (5 + l.rng.pick(2), 6 + l.rng.pick(3));
    defences(l, &track, middle, pits, boxes);
}

/// Hardstands off the perimeter track, outward: round frying pans, and spectacles (two side by side on one stub); none
/// near where a runway meets the track.
fn hardstands(l: &mut Layout<'_, '_>, track: &[Vec2], middle: Vec2, ends: &[Vec2]) {
    let along = distances(track);
    let total = *along.last().expect("a track");
    let want = 30 + l.rng.pick(7);
    let step = total / want as f32;
    let mut at = l.rng.range(0.0, step);
    let mut laid = 0;
    while at < total && laid < want + 2 {
        let (q, tangent) = point_at(track, &along, at);
        at += step * l.rng.range(0.85, 1.15);
        if ends.iter().any(|e| e.distance(q) < 110.0) {
            continue;
        }
        let mut out = perp(tangent);
        if out.dot(q - middle) < 0.0 {
            out = -out;
        }
        let out = rotate(out, l.rng.signed() * 12.0);
        let spectacle = l.rng.chance(0.35);
        let stub = l.rng.range(14.0, 28.0);
        let base = q + out * TRACK_HALF_M;
        let pans: Vec<Vec2> = if spectacle {
            let c = base + out * (stub + 0.8 * PAN_M);
            vec![c + tangent * (PAN_M + 2.0), c - tangent * (PAN_M + 2.0)]
        } else {
            vec![base + out * (stub + PAN_M)]
        };
        let fits = pans.iter().all(|p| {
            l.fair(*p, PAN_M + 2.0, 0.06)
                && l.off_strips(*p) > PAN_M + 4.0
                && l.off_pieces(*p) > PAN_M + 8.0
        });
        if !fits {
            continue;
        }
        let tip = base + out * (stub + 4.0);
        l.plan.ribbons.push(Ribbon {
            points: vec![q, tip],
            half: 5.5,
            paved: 5.5,
            look: Look::Tarmac,
            surface: Surface::Coral,
            lift: TAXIWAY_LIFT - 0.006,
            across: 2,
        });
        for p in pans {
            let ring = rough(&mut l.rng, out, PAN_M, PAN_M, 2.0, 0.015, 24);
            l.plan.pads.push(Pad {
                centre: p,
                ring,
                look: Look::TarmacPad,
                lift: PAD_LIFT,
            });
            l.plan.clearing.push(Capsule {
                a: q,
                b: p,
                radius: PAN_M + 12.0,
                soft: false,
            });
            l.plan.stands += 1;
            laid += 1;
            if l.rng.chance(l.fill) {
                let facing = rotate(-out, l.rng.signed() * 40.0);
                l.park(p, facing, PAN_M, PAD_LIFT);
            }
        }
    }
}

/// A T2 hangar at `at`, its doors toward `toward` (and away), its apron of tarmac before it.
fn t2(l: &mut Layout<'_, '_>, at: Vec2, toward: Vec2) {
    if !l.fair(at, 40.0, 0.05) || l.off_pieces(at) < 45.0 || l.off_strips(at) < 40.0 {
        return;
    }
    let toward = rotate(toward, l.rng.signed() * 4.0);
    let colour = T2_ROOF[l.rng.pick(T2_ROOF.len())];
    let ends = T2_WALLS[l.rng.pick(T2_WALLS.len())];
    l.plan.huts.push(Hut {
        centre: at,
        axis: toward,
        length: 73.0,
        half: 17.5,
        height: 9.0,
        kind: HutKind::T2,
        colour,
        ends,
    });
    let apron = at + toward * (36.5 + 22.0);
    let ring = rough(&mut l.rng, toward, 20.0, 26.0, 5.0, 0.02, 20);
    l.plan.pads.push(Pad {
        centre: apron,
        ring,
        look: Look::TarmacPad,
        lift: LOT_LIFT,
    });
    l.plan.clearing.push(Capsule {
        a: at,
        b: apron,
        radius: 48.0,
        soft: false,
    });
}

/// The watch office at `at`, facing the runway (away from `tech`); `small` a fighter station's.
fn watch_office(l: &mut Layout<'_, '_>, at: Vec2, tech: f32, small: bool) {
    let s = l.site;
    if !l.fair(at, 10.0, 0.08) || l.off_strips(at) < 10.0 {
        return;
    }
    let toward = -tech * s.right();
    l.plan.huts.push(Hut {
        centre: at,
        axis: toward,
        length: if small { 8.0 } else { 10.0 },
        half: if small { 5.0 } else { 7.0 },
        height: 6.4,
        kind: HutKind::WatchOffice,
        colour: SLATE[0],
        ends: RENDER[l.rng.pick(RENDER.len())],
    });
    // A van and a fire tender by it.
    for k in 0..2 {
        let p = at + s.dir * (12.0 + 6.0 * k as f32) - toward * 4.0;
        if l.off_pieces(p) > 3.0 {
            l.plan.vehicles.push(Vehicle {
                centre: p,
                axis: rotate(s.dir, l.rng.signed() * 20.0),
                kind: if k == 0 {
                    VehicleKind::Carrier
                } else {
                    VehicleKind::Truck
                },
                colour: RAF_BLUE[l.rng.pick(RAF_BLUE.len())],
            });
        }
    }
}

/// A Nissen hut (16 ft across, its ridge along `axis`) or a brick building at `at` along `axis`, if it fits.
pub(super) fn building(l: &mut Layout<'_, '_>, at: Vec2, axis: Vec2, brick: bool) -> bool {
    let (length, half) = if brick {
        (l.rng.range(14.0, 30.0), l.rng.range(4.0, 6.0))
    } else {
        (l.rng.range(9.0, 22.0), 2.45)
    };
    let ends = [at - axis * (0.5 * length), at, at + axis * (0.5 * length)];
    if !l.fair(at, 0.5 * length, 0.1)
        || !ends
            .iter()
            .all(|p| l.dry(*p) && l.off_strips(*p) > half + 2.5 && l.off_pieces(*p) > half + 3.0)
    {
        return false;
    }
    let (kind, height, colour, ends) = if brick {
        (
            HutKind::Brick,
            5.0,
            SLATE[l.rng.pick(SLATE.len())],
            BRICK[l.rng.pick(BRICK.len())],
        )
    } else {
        (
            HutKind::Quonset,
            2.5,
            NISSEN[l.rng.pick(NISSEN.len())],
            BRICK[l.rng.pick(BRICK.len())],
        )
    };
    l.plan.huts.push(Hut {
        centre: at,
        axis,
        length,
        half,
        height,
        kind,
        colour,
        ends,
    });
    true
}

/// The technical site about `centre`, `away` from the airfield: a lane in from the track, workshops and stores in
/// brick and Nissen huts along roads of their own at slightly different angles, a water tower, fuel tanks.
fn technical_site(l: &mut Layout<'_, '_>, centre: Vec2, away: Vec2) {
    let s = l.site;
    if !l.dry(centre) {
        return;
    }
    let along = rotate(s.dir, l.rng.signed() * 8.0);
    // Its two roads, side by side across it, each joined to the track.
    for row in [-1.0f32, 1.0] {
        let mid = centre + away * (row * 40.0);
        let (a, b) = (mid - along * 150.0, mid + along * 150.0);
        let bend = (a + b) * 0.5 + away * l.rng.signed() * 6.0;
        if !l.road_along(&[a, bend, b], 3.0, Look::Tarmac) {
            continue;
        }
        let mut u = -130.0;
        while u < 130.0 {
            let side = if l.rng.chance(0.5) { 1.0 } else { -1.0 };
            let brick = l.rng.chance(0.4);
            let turn = l.rng.signed() * 6.0;
            let at = mid + along * u + away * (side * l.rng.range(14.0, 22.0));
            if l.rng.chance(0.85) && building(l, at, rotate(along, turn), brick) {
                u += l.rng.range(20.0, 34.0);
            } else {
                u += 10.0;
            }
        }
    }
    l.join(centre, 60.0, 3.0);
    // The water tower and the fuel tanks.
    let tower = centre - away * 80.0 + along * l.rng.signed() * 100.0;
    if l.fair(tower, 5.0, 0.1) && l.off_pieces(tower) > 6.0 && l.off_strips(tower) > 6.0 {
        l.plan.tanks.push(Tank {
            centre: tower,
            radius: 3.2,
            stand: l.rng.range(9.0, 12.0),
            height: 3.5,
            tower: true,
        });
    }
    for k in 0..2 + l.rng.pick(2) {
        let p = centre + along * (170.0 + 12.0 * k as f32) + away * l.rng.signed() * 10.0;
        if l.fair(p, 5.0, 0.08) && l.off_pieces(p) > 4.0 && l.off_strips(p) > 5.0 {
            l.plan.tanks.push(Tank {
                centre: p,
                radius: 4.0,
                stand: 0.6,
                height: 3.2,
                tower: false,
            });
        }
    }
    for _ in 0..3 {
        let p = centre + along * l.rng.signed() * 120.0 + away * l.rng.signed() * 30.0;
        if l.off_pieces(p) > 4.0 && l.off_strips(p) > 3.0 && l.dry(p) {
            l.plan.vehicles.push(Vehicle {
                centre: p,
                axis: rotate(along, l.rng.range(0.0, 360.0)),
                kind: if l.rng.chance(0.3) {
                    VehicleKind::Bowser
                } else {
                    VehicleKind::Truck
                },
                colour: RAF_BLUE[l.rng.pick(RAF_BLUE.len())],
            });
        }
    }
}

/// `count` accommodation sites out among the fields `away` from the airfield's `middle` (within 60°), each Nissen huts
/// in two or three rows with a brick ablutions block, a lane back to the roads.
fn accommodation(l: &mut Layout<'_, '_>, middle: Vec2, away: Vec2, count: usize) {
    let mut laid = 0;
    for _ in 0..count * 8 {
        if laid >= count {
            break;
        }
        let dir = rotate(away, l.rng.signed() * 60.0);
        let centre = middle + dir * l.rng.range(900.0, 1600.0);
        if !l.fair(centre, 50.0, 0.08)
            || l.off_strips(centre) < 60.0
            || l.off_pieces(centre) < 120.0
        {
            continue;
        }
        let axis = rotate(perp(dir), l.rng.signed() * 25.0);
        let rows = 2 + l.rng.pick(2);
        let per = 3 + l.rng.pick(4);
        let mut stood = 0;
        for r in 0..rows {
            for k in 0..per {
                let at = centre
                    + axis * ((k as f32 - 0.5 * (per - 1) as f32) * 13.0)
                    + perp(axis) * ((r as f32 - 0.5 * (rows - 1) as f32) * 20.0)
                    + Vec2::new(l.rng.signed(), l.rng.signed()) * 1.0;
                if l.rng.chance(0.9) && building(l, at, perp(axis), false) {
                    stood += 1;
                }
            }
        }
        let block = centre + perp(axis) * (0.5 * rows as f32 * 20.0 + 14.0);
        building(l, block, axis, true);
        if stood > 0 {
            l.join(centre, 45.0, 2.5);
            laid += 1;
        }
    }
}

/// The bomb dump about `at`: a loop of lane, earth traverses between its stores, bombs stacked in their bays.
fn bomb_dump(l: &mut Layout<'_, '_>, at: Vec2) {
    let mut centre = None;
    for k in 0..8 {
        let p = at + Vec2::from_angle(k as f32 * TAU / 8.0) * (k as f32 * 60.0);
        if l.fair(p, 70.0, 0.06) && l.off_strips(p) > 80.0 && l.off_pieces(p) > 80.0 {
            centre = Some(p);
            break;
        }
    }
    let Some(centre) = centre else { return };
    let axis = rotate(l.site.dir, l.rng.range(0.0, 180.0));
    let loop_points: Vec<Vec2> = (0..8)
        .map(|k| {
            let t = k as f32 * TAU / 8.0;
            centre + axis * (t.cos() * 60.0) + perp(axis) * (t.sin() * 35.0)
        })
        .collect();
    let lane = smooth_loop(&loop_points, 2);
    if !l.road_along(&lane, 3.0, Look::Tarmac) {
        return;
    }
    let soil = SOIL[l.rng.pick(SOIL.len())];
    let along = distances(&lane);
    let total = *along.last().expect("a lane");
    let mut s = 10.0;
    while s < total - 10.0 {
        let (q, tangent) = point_at(&lane, &along, s);
        let mut out = perp(tangent);
        if out.dot(q - centre) < 0.0 {
            out = -out;
        }
        // A bay: a traverse either side, the bombs between.
        let bay = q + out * 14.0;
        for side in [-1.0f32, 1.0] {
            let c = bay + tangent * (side * 9.0);
            let seed = l.rng.next_u64() as u32;
            l.plan.banks.push(Bank {
                a: c - out * 8.0,
                b: c + out * 8.0,
                height: 2.4,
                berm: 2.6,
                colour: soil,
                seed,
            });
        }
        for k in 0..2 {
            l.plan.crates.push(Crates {
                centre: bay + out * (k as f32 * 4.0 - 2.0),
                axis: tangent,
                half: Vec2::new(l.rng.range(1.8, 2.6), 1.2),
                height: l.rng.range(0.8, 1.2),
                colour: [70, 74, 62],
            });
        }
        s += l.rng.range(34.0, 44.0);
    }
    l.join(centre, 70.0, 3.0);
}

/// Bofors pits and pillboxes about the airfield, outside its track.
fn defences(l: &mut Layout<'_, '_>, track: &[Vec2], middle: Vec2, pits: usize, boxes: usize) {
    let along = distances(track);
    let total = *along.last().expect("a track");
    for k in 0..pits + boxes {
        let (q, _) = point_at(
            track,
            &along,
            (k as f32 + l.rng.unit()) / (pits + boxes) as f32 * total,
        );
        let out = (q - middle).normalize_or(Vec2::X);
        let p = q + out * l.rng.range(45.0, 90.0);
        if !l.fair(p, 5.0, 0.15) || l.off_strips(p) < 8.0 || l.off_pieces(p) < 10.0 {
            continue;
        }
        if k % 2 == 0 && l.plan.pits.len() < pits {
            l.plan.pits.push(Pit {
                centre: p,
                radius: 2.9,
                opening: -out,
                gun: Gun::Light,
                aim: rotate(out, l.rng.signed() * 50.0),
            });
        } else {
            l.plan.pillboxes.push(Pillbox {
                centre: p,
                axis: out,
                radius: 2.3,
                height: 1.7,
            });
        }
    }
}

/// The grass fighter station: its landing runs mown across the field, a perimeter track round it, E-shaped blast pens
/// and blister hangars off the track, the watch office, Nissen huts beyond.
fn grass_field(l: &mut Layout<'_, '_>) {
    let s = l.site;
    let gravel = l.input.gravel;
    // The field: grass mown about the runs, some 1,300 by 900 m.
    let (rx, ry) = (0.5 * s.length + 100.0, l.rng.range(380.0, 460.0));
    l.plan.clearing.push(Capsule {
        a: s.at(-0.5 * s.length + 300.0, 0.0),
        b: s.at(0.5 * s.length - 300.0, 0.0),
        radius: ry,
        soft: false,
    });
    bulges(l, Vec2::new(rx, ry));
    let [e0, e1] = s.ends();
    let (look, half) = if gravel {
        (Look::Field, 25.0)
    } else {
        (Look::Mown, s.half)
    };
    l.plan.ribbons.push(Ribbon {
        points: vec![e0, e1],
        half,
        paved: half,
        look,
        surface: Surface::Coral,
        lift: RUNWAY_LIFT,
        across: 4,
    });
    // A second run across it.
    let turn = l.rng.range(50.0, 80.0) * if l.rng.chance(0.5) { 1.0 } else { -1.0 };
    let d = rotate(s.dir, turn);
    let run = l.rng.range(800.0, 950.0);
    let (a, b) = (s.centre - d * (0.5 * run), s.centre + d * (0.5 * run));
    if runs_fair(l, a, b, 20.0) {
        l.plan.ribbons.push(Ribbon {
            points: vec![a, b],
            half: half * 0.9,
            paved: half * 0.9,
            look,
            surface: Surface::Coral,
            lift: RUNWAY_LIFT - 0.004,
            across: 4,
        });
        l.plan.clearing.push(Capsule {
            a,
            b,
            radius: half + 40.0,
            soft: false,
        });
    }
    // The perimeter track: round the field just inside its edge.
    let ring: Vec<Vec2> = (0..12)
        .map(|k| {
            let t = k as f32 * TAU / 12.0;
            let wobble = 1.0 + 0.04 * l.rng.signed();
            s.at(t.cos() * rx * wobble, t.sin() * (ry - 40.0) * wobble)
        })
        .collect();
    let track = smooth_loop(&ring, 3);
    if track.iter().all(|p| l.dry(*p)) {
        paved(l, track.clone(), 6.0, Look::Tarmac, TAXIWAY_LIFT, 20.0);
    }
    let middle = s.centre;
    l.soft = true;
    let along = distances(&track);
    let total = *along.last().expect("a track");
    // Round the track: pens and blisters by turns, gaps between.
    let soil = SOIL[l.rng.pick(SOIL.len())];
    let mut at = l.rng.range(0.0, 80.0);
    let mut k = 0;
    while at < total {
        let (q, tangent) = point_at(&track, &along, at);
        at += l.rng.range(110.0, 170.0);
        let mut out = perp(tangent);
        if out.dot(q - middle) < 0.0 {
            out = -out;
        }
        // Not across a run's end.
        if l.off_strips(q + out * 30.0) < 20.0 {
            continue;
        }
        k += 1;
        if k % 3 == 0 {
            let centre = q + out * (6.0 + 6.0 + 9.0);
            if l.fair(centre, 14.0, 0.08) && l.off_pieces(centre) > 16.0 {
                l.plan.huts.push(Hut {
                    centre,
                    axis: out,
                    length: l.rng.range(14.0, 20.0),
                    half: 10.5,
                    height: 6.0,
                    kind: HutKind::Blister,
                    colour: NISSEN[l.rng.pick(NISSEN.len())],
                    ends: NISSEN[0],
                });
            }
        } else {
            e_pen(l, q, tangent, out, soil);
        }
    }
    // The watch office by the track on its nearer side, Nissen huts beyond it.
    let tech = -s.side;
    let shift = l.rng.signed() * 0.1 * s.length;
    watch_office(l, s.at(shift, tech * (ry - 90.0)), tech, true);
    for g in 0..2 + l.rng.pick(2) {
        let centre = s.at(
            (g as f32 - 1.0) * 220.0 + l.rng.signed() * 40.0,
            tech * (ry + l.rng.range(60.0, 160.0)),
        );
        let axis = rotate(s.dir, l.rng.signed() * 20.0);
        let mut stood = 0;
        for r in 0..2 {
            for i in 0..3 + l.rng.pick(3) {
                let at = centre + axis * (i as f32 * 13.0 - 20.0) + perp(axis) * (r as f32 * 18.0);
                let brick = l.rng.chance(0.15);
                if building(l, at, perp(axis), brick) {
                    stood += 1;
                }
            }
        }
        if stood > 0 {
            l.join(centre, 40.0, 2.5);
        }
    }
    let (pits, boxes) = (3 + l.rng.pick(2), 4 + l.rng.pick(3));
    defences(l, &track, middle, pits, boxes);
}

/// An E-shaped blast pen off the track at `q`: two bays opening onto it, earth walls about them (its back, its two ends
/// and a wall between), a plane in a bay now and then.
fn e_pen(l: &mut Layout<'_, '_>, q: Vec2, tangent: Vec2, out: Vec2, soil: [u8; 3]) {
    let bay = 15.0;
    let depth = 14.0;
    let front = q + out * 14.0;
    let back = front + out * depth;
    let centre = (front + back) * 0.5;
    if !l.fair(centre, bay + 4.0, 0.08)
        || l.off_pieces(centre) < bay + 10.0
        || l.off_strips(centre) < 0.5 * depth + 2.0
    {
        return;
    }
    let height = l.rng.range(2.6, 3.2);
    let wall = |l: &mut Layout<'_, '_>, a: Vec2, b: Vec2| {
        let seed = l.rng.next_u64() as u32;
        l.plan.banks.push(Bank {
            a,
            b,
            height,
            berm: 2.4,
            colour: soil,
            seed,
        });
    };
    wall(
        l,
        back - tangent * (bay + 2.0),
        back + tangent * (bay + 2.0),
    );
    for x in [-1.0f32, 0.0, 1.0] {
        let foot = back + tangent * (x * (bay + 2.0));
        wall(l, foot, foot - out * (depth - 1.0));
    }
    for x in [-0.5f32, 0.5] {
        l.plan.stands += 1;
        if l.rng.chance(l.fill) {
            let p = centre + tangent * (x * (bay + 2.0));
            l.park(p, -out, 0.5 * bay + 1.0, 0.0);
        }
    }
    l.plan.clearing.push(Capsule {
        a: q,
        b: back,
        radius: bay + 8.0,
        soft: true,
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::site::fit;
    use super::super::{Template, lay_out};
    use super::*;

    /// Farmland: a gentle plain rising to the east, a brook across it 2.5 km north.
    pub(crate) fn farmland(p: Vec2) -> f32 {
        8.0 + 0.002 * p.x + 0.6 * (p.y * 0.004).sin() + 0.3 * (p.x * 0.011).sin()
    }

    pub(crate) fn brook(p: Vec2) -> bool {
        (p.y + 2500.0).abs() < 8.0
    }

    fn plan(size: SizeClass, seed: u32) -> Plan {
        let input = SiteInput {
            seed,
            size,
            template: Template::RafStation,
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
    fn an_raf_station_is_laid_out_alike_every_time_and_on_dry_ground() {
        for size in [SizeClass::BomberField, SizeClass::FighterStrip] {
            let a = plan(size, 7);
            assert_eq!(a, plan(size, 7), "laid out alike");
            assert!(a.temperate);
            let runways = a
                .ribbons
                .iter()
                .filter(|r| r.lift >= RUNWAY_LIFT - 0.01)
                .count();
            if size == SizeClass::BomberField {
                assert_eq!(runways, 3, "a Class A's three runways");
                assert!(
                    a.ribbons
                        .iter()
                        .all(|r| !matches!(r.look, Look::Runway | Look::Field))
                );
                assert_eq!(a.huts.iter().filter(|h| h.kind == HutKind::T2).count(), 2);
                assert!(a.stands >= 24, "{} hardstands", a.stands);
                assert!(a.pads.iter().filter(|p| p.look == Look::TarmacPad).count() >= 24);
                assert!(a.banks.len() >= 6, "a bomb dump's traverses");
            } else {
                assert!(runways >= 1);
                assert!(a.ribbons.iter().any(|r| r.look == Look::Mown));
                assert!(a.huts.iter().any(|h| h.kind == HutKind::Blister));
                assert!(a.banks.len() >= 8, "E pens: {}", a.banks.len());
            }
            assert!(a.huts.iter().any(|h| h.kind == HutKind::WatchOffice));
            assert!(
                a.huts.iter().filter(|h| h.kind == HutKind::Quonset).count() >= 10,
                "Nissen huts"
            );
            assert!(!a.pillboxes.is_empty() && !a.pits.is_empty());
            // No types of its own: nothing parked.
            assert!(a.parked.is_empty());
            for p in a
                .huts
                .iter()
                .map(|h| h.centre)
                .chain(a.pads.iter().map(|p| p.centre))
                .chain(a.banks.iter().map(|b| (b.a + b.b) * 0.5))
                .chain(a.pits.iter().map(|p| p.centre))
                .chain(a.pillboxes.iter().map(|p| p.centre))
            {
                assert!(!brook(p), "{p} wet");
                assert!(a.holds(p), "{p} outside the clearing");
            }
        }
        assert_ne!(
            plan(SizeClass::BomberField, 7).huts,
            plan(SizeClass::BomberField, 8).huts
        );
    }
}
