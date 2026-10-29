//! The terrain review's fixed views of each map (`scripts/diagnostics/terrain-review.ts`), found from the field
//! exactly as the web page finds them, so native stills and the web's captures share their cameras: the coast that
//! rises highest over the sea from the lane's centre ("the main coast") from sea level at 3, 8 and 20 km, through 12×
//! binoculars and from an aircraft; the steepest coast from 2.5 km and through 4× from 900 m; and the whole chart
//! from 30 km up. (The page's `spawn` and `art` views follow the player's ship and are not ported.)

use crate::heightfield::Heightfield;

/// A camera in world metres (Y up), with its vertical field of view and far plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewCamera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    /// Degrees.
    pub fov: f64,
    /// Metres; `None` keeps the battle's.
    pub far: Option<f64>,
}

/// The presets this port knows, in the page's order.
pub const PRESETS: [&str; 8] = [
    "coast3",
    "coast8",
    "coast20",
    "binoculars",
    "aerial",
    "cliffs",
    "cliffsNear",
    "overview",
];

/// The coast that stands highest over the sea seen from the lane's centre (`mainCoast`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainCoast {
    pub direction: [f64; 2],
    pub hit: f64,
    pub angle: f64,
}

pub fn main_coast(field: &Heightfield) -> MainCoast {
    let mut best = MainCoast {
        direction: [0.0, -1.0],
        hit: 8000.0,
        angle: 0.0,
    };
    let mut best_score = f64::NEG_INFINITY;
    for degrees in (0..360).step_by(2) {
        let a = f64::from(degrees) * std::f64::consts::PI / 180.0;
        let (dx, dz) = (a.sin(), -a.cos());
        let mut hit = 0.0;
        let mut d = 2000.0;
        while d < 32000.0 {
            if field.height(dx * d, dz * d) > 3.0 {
                hit = d;
                break;
            }
            d += 50.0;
        }
        if hit == 0.0 {
            continue;
        }
        let mut angle: f64 = 0.0;
        let mut d = hit;
        while d < hit + 16000.0 {
            angle = angle.max((field.height(dx * d, dz * d) - 25.0) / d);
            d += 100.0;
        }
        let score = angle / (1.0 + hit / 12000.0);
        if score > best_score {
            best_score = score;
            best = MainCoast {
                direction: [dx, dz],
                hit,
                angle,
            };
        }
    }
    best
}

/// The land sample within 400 m of the sea (and 30 km of the lane) whose height times slope is greatest, and the unit
/// direction down its slope toward the sea (`steepestCoast`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteepCoast {
    pub x: f64,
    pub z: f64,
    pub dx: f64,
    pub dz: f64,
    pub height: f64,
}

pub fn steepest_coast(field: &Heightfield) -> SteepCoast {
    let mut best = SteepCoast {
        x: 0.0,
        z: 0.0,
        dx: 0.0,
        dz: 1.0,
        height: 0.0,
    };
    let mut best_score = f64::NEG_INFINITY;
    let h = |x: f64, z: f64| field.height(x, z);
    let mut z = -30000.0;
    while z <= 30000.0 {
        let mut x = -30000.0;
        while x <= 30000.0 {
            let here = h(x, z);
            if here >= 20.0 {
                let gx = (h(x + 40.0, z) - h(x - 40.0, z)) / 80.0;
                let gz = (h(x, z + 40.0) - h(x, z - 40.0)) / 80.0;
                let slope = gx.hypot(gz);
                if slope >= 0.3 {
                    let (dx, dz) = (-gx / slope, -gz / slope);
                    let inland = h(x + dx * 400.0, z + dz * 400.0) > 0.0
                        && h(x + dx * 250.0, z + dz * 250.0) > 0.0;
                    if !inland {
                        let mut open = true;
                        let mut d = 500.0;
                        while d <= 2600.0 && open {
                            open = h(x + dx * d, z + dz * d) < -1.0;
                            d += 100.0;
                        }
                        let spike = h(x - dx * 160.0, z - dz * 160.0) < here * 0.5
                            || h(x - dx * 80.0 + dz * 80.0, z - dz * 80.0 - dx * 80.0) < here * 0.5;
                        let score = here * slope;
                        if open && !spike && score > best_score {
                            best_score = score;
                            best = SteepCoast {
                                x,
                                z,
                                dx,
                                dz,
                                height: here,
                            };
                        }
                    }
                }
            }
            x += 80.0;
        }
        z += 80.0;
    }
    best
}

/// A point `back` metres seaward of the coast along the main bearing, stepped out until it is over water (`seaward`).
fn seaward(field: &Heightfield, coast: &MainCoast, back: f64) -> [f64; 2] {
    let [dx, dz] = coast.direction;
    let mut d = coast.hit - back;
    loop {
        if field.height(dx * d, dz * d) < -2.0 || d < coast.hit - back - 20000.0 {
            return [dx * d, dz * d];
        }
        d -= 100.0;
    }
}

/// The lens of `magnification` × binoculars on the game's 52° camera (`zoom`).
pub fn zoom(magnification: f64) -> f64 {
    2.0 * ((52.0 * std::f64::consts::PI / 360.0).tan() / magnification).atan() * 180.0
        / std::f64::consts::PI
}

/// The view `preset` on `field`, placed in a battle whose chart sits at `offset` (world = chart + offset). `None` for
/// a preset this port does not know.
pub fn preset(field: &Heightfield, offset: [f64; 2], preset: &str) -> Option<ReviewCamera> {
    let world = |[x, z]: [f64; 2], y: f64| [x + offset[0], y, z + offset[1]];
    let look = |eye: [f64; 3], target: [f64; 3], fov: f64, far: Option<f64>| ReviewCamera {
        eye,
        target,
        fov,
        far,
    };
    let vista = |back: f64| {
        let coast = main_coast(field);
        let [dx, dz] = coast.direction;
        let [x, z] = seaward(field, &coast, back);
        let mut crest: f64 = 0.0;
        let mut d = 100.0;
        while d < 40000.0 {
            crest = crest.max((field.height(x + dx * d, z + dz * d) - 25.0) / d);
            d += 100.0;
        }
        let pitch = (crest * 0.4).min(0.1);
        look(
            world([x, z], 25.0),
            world([x + dx * 4000.0, z + dz * 4000.0], 25.0 + 4000.0 * pitch),
            52.0,
            None,
        )
    };
    Some(match preset {
        "coast3" => vista(3000.0),
        "coast8" => vista(8000.0),
        "coast20" => vista(20000.0),
        "binoculars" => {
            let coast = main_coast(field);
            let [dx, dz] = coast.direction;
            let d = coast.hit + 400.0;
            look(
                world([0.0, 0.0], 30.0),
                world(
                    [dx * d, dz * d],
                    field.height(dx * d, dz * d).max(0.0) + 25.0,
                ),
                zoom(12.0),
                None,
            )
        }
        "aerial" => {
            let coast = main_coast(field);
            let [dx, dz] = coast.direction;
            let [x, z] = seaward(field, &coast, 3000.0);
            look(
                world([x, z], 900.0),
                world([x + dx * 5500.0, z + dz * 5500.0], 0.0),
                52.0,
                None,
            )
        }
        "cliffs" => {
            let s = steepest_coast(field);
            look(
                world([s.x + s.dx * 2500.0, s.z + s.dz * 2500.0], 20.0),
                world([s.x, s.z], s.height * 0.45),
                40.0,
                None,
            )
        }
        "cliffsNear" => {
            let s = steepest_coast(field);
            look(
                world([s.x + s.dx * 900.0, s.z + s.dz * 900.0], 12.0),
                world([s.x, s.z], s.height * 0.4),
                zoom(4.0),
                None,
            )
        }
        "overview" => look(
            world([0.0, 4000.0], 30000.0),
            world([0.0, 0.0], 0.0),
            70.0,
            Some(200000.0),
        ),
        _ => return None,
    })
}
