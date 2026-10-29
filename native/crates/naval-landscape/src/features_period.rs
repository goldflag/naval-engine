//! The period's roads on the theatre's land (native/docs/terrain/period.md, "The period's ground", "The roads"): which
//! of today's ways a past period ([`Period::Year`]) draws on the features' pages and the bridges, and as what.
//! [`Period::Today`] draws every way as [`super::style_of`] has it, untouched.
//!
//! - **Motorways** and their ramps only where a section stood ([`MOTORWAYS`], by place and year): a way is kept where
//!   it runs along a listed section's route (within [`CORRIDOR_M`], within [`ALONG_DEG`] of its heading; a ramp within
//!   [`RAMP_CORRIDOR_M`] whatever its heading). A section that was an ordinary road before it was made a motorway is
//!   drawn as a trunk road then, without its ramps. Busways are later than every period.
//! - **Streets** (the minor classes, [`minor`]: OpenMapTiles' `minor`, which folds residential and unclassified roads,
//!   living streets and plain roads together, and `service`, `raceway`) are drawn only where the ground stood
//!   (`period_land::stood`): each way sampled every [`STEP_M`] along it, cut where the answer changes (to a metre or
//!   two), the gaps under [`GAP_M`] closed and what is left at a cut shorter than [`STUB_M`] (with a cut at both ends,
//!   [`ISLAND_M`]) dropped, so a street ends where its town ended rather than as a stub into the country. The
//!   arterials (trunk to tertiary), tracks, railways and waterways are the period's as they are.
//! - **Paving** ([`paving`]): where the evidence says the period's minor roads were earth or gravel, they are drawn
//!   unpaved (and so take no street lights).
//! - **Bridges** built after the period (`naval_terrain::infra::period`) are not drawn, nor their roads.

use std::hash::{Hash, Hasher};

use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_osm::{Brunnel, Geometry, Transportation};
use naval_terrain::StrokeKind;
use naval_terrain::buildings::Period;
use naval_terrain::infra::period as infra_period;

use super::osm::OsmTile;

/// A motorway's section: its route as waypoints (latitude, longitude) along its carriageways' middle, the year it
/// opened as a motorway, and the year it stood as an ordinary road before that (none: built as a motorway).
pub struct Section {
    pub name: &'static str,
    pub year: u16,
    pub road: Option<u16>,
    pub route: &'static [(f64, f64)],
}

/// Motorways that stood in a period, by place and the year each section opened (a section that opened late in a year
/// counts from the next). Nowhere else in the theatres' regions did a grade-separated road stand before 1948: Los
/// Angeles' next are the Santa Ana Freeway's first section and the Terminal Island Freeway (1948), Hawaii's the
/// Mauka Arterial's first section (1953), Japan's the Shuto (1962) and the Meishin (1963), southern England's the
/// Chiswick flyover (1959) and the M2 (1963); Iceland and the Solomons have none. The list is whole only to 1947: a
/// later period draws the sections here and none of Los Angeles' other freeways.
pub const MOTORWAYS: &[Section] = &[
    // The Arroyo Seco Parkway (SR 110): Glenarm Street to Fair Oaks Avenue opened on 10 December 1938, on to the
    // Figueroa Street viaduct by 30 December 1940 (South Pasadena's part on 30 January 1941), the extension through the
    // Figueroa Street tunnels (a city street from 1937) to College Street in December 1943, to the Four Level in 1953.
    Section {
        name: "Arroyo Seco Parkway, Glenarm Street to Fair Oaks Avenue",
        year: 1939,
        road: None,
        route: &[
            (34.1275, -118.1472),
            (34.1252, -118.1466),
            (34.1224, -118.1460),
            (34.1199, -118.1473),
            (34.1191, -118.1505),
        ],
    },
    Section {
        name: "Arroyo Seco Parkway, Fair Oaks Avenue to the Figueroa Street viaduct",
        year: 1941,
        road: None,
        route: &[
            (34.1191, -118.1505),
            (34.1191, -118.1548),
            (34.1192, -118.1596),
            (34.1188, -118.1633),
            (34.1180, -118.1674),
            (34.1163, -118.1709),
            (34.1139, -118.1748),
            (34.1125, -118.1779),
            (34.1111, -118.1810),
            (34.1098, -118.1849),
            (34.1065, -118.1877),
            (34.1041, -118.1911),
            (34.1028, -118.1943),
            (34.1027, -118.1978),
            (34.1013, -118.2007),
            (34.0983, -118.2034),
            (34.0943, -118.2062),
            (34.0915, -118.2068),
            (34.0888, -118.2088),
            (34.0865, -118.2122),
            (34.0860, -118.2157),
            (34.0846, -118.2198),
            (34.0828, -118.2226),
            (34.0803, -118.2259),
        ],
    },
    Section {
        name: "Arroyo Seco Parkway, the Figueroa Street tunnels to College Street",
        year: 1944,
        road: Some(1937),
        route: &[
            (34.0803, -118.2259),
            (34.0792, -118.2275),
            (34.0771, -118.2297),
            (34.0747, -118.2325),
            (34.0727, -118.2348),
            (34.0702, -118.2359),
            (34.0680, -118.2381),
            (34.0663, -118.2413),
            (34.0656, -118.2427),
        ],
    },
    Section {
        name: "Arroyo Seco Parkway, College Street to the Four Level",
        year: 1953,
        road: None,
        route: &[
            (34.0656, -118.2427),
            (34.0650, -118.2443),
            (34.0637, -118.2477),
            (34.0627, -118.2488),
        ],
    },
    // The Cahuenga Pass Freeway (US 101), Highland Avenue to Barham Boulevard, opened in stages in 1940 and 1941; on
    // to Vineland Avenue in 1949.
    Section {
        name: "Cahuenga Pass Freeway, Highland Avenue to Barham Boulevard",
        year: 1941,
        road: None,
        route: &[
            (34.1099, -118.3337),
            (34.1154, -118.3366),
            (34.1210, -118.3396),
            (34.1251, -118.3440),
            (34.1277, -118.3458),
            (34.1294, -118.3475),
        ],
    },
    Section {
        name: "Hollywood Freeway, Barham Boulevard to Vineland Avenue",
        year: 1949,
        road: None,
        route: &[
            (34.1294, -118.3475),
            (34.1306, -118.3490),
            (34.1330, -118.3518),
            (34.1339, -118.3549),
            (34.1346, -118.3580),
            (34.1364, -118.3608),
            (34.1389, -118.3643),
            (34.1410, -118.3665),
            (34.1441, -118.3679),
        ],
    },
    // The Ramona Parkway (I-10), on Ramona Boulevard's line: a boulevard from 1935, a freeway in the city of Los
    // Angeles from 1944 and on to Alhambra by 1953.
    Section {
        name: "Ramona Parkway, the Los Angeles River to Indiana Street",
        year: 1944,
        road: Some(1935),
        route: &[
            (34.0553, -118.2159),
            (34.0556, -118.2119),
            (34.0544, -118.2089),
            (34.0535, -118.2056),
            (34.0552, -118.2030),
            (34.0558, -118.1997),
            (34.0556, -118.1965),
            (34.0555, -118.1915),
        ],
    },
    Section {
        name: "Ramona Freeway, Indiana Street to Alhambra",
        year: 1953,
        road: Some(1935),
        route: &[
            (34.0555, -118.1915),
            (34.0557, -118.1889),
            (34.0553, -118.1857),
            (34.0567, -118.1829),
            (34.0580, -118.1800),
            (34.0591, -118.1766),
            (34.0609, -118.1741),
            (34.0621, -118.1712),
            (34.0625, -118.1680),
            (34.0626, -118.1624),
            (34.0641, -118.1597),
            (34.0659, -118.1561),
            (34.0676, -118.1531),
            (34.0690, -118.1502),
            (34.0707, -118.1473),
            (34.0718, -118.1443),
            (34.0713, -118.1399),
        ],
    },
];

/// How far a motorway's way may lie from a listed route (m), and a ramp's.
pub const CORRIDOR_M: f64 = 120.0;
pub const RAMP_CORRIDOR_M: f64 = 90.0;
/// How far a main way's heading may turn from its route's (degrees; either way along it).
pub const ALONG_DEG: f64 = 30.0;
/// The step a way is sampled at (m), and the precision its cuts are found to.
pub const STEP_M: f64 = 20.0;
const CUT_M: f64 = 1.5;
/// A gap closed between kept runs (m): the kept land's edge wanders.
pub const GAP_M: f64 = 40.0;
/// The least run kept at a cut (m), and with cuts at both ends.
pub const STUB_M: f64 = 60.0;
pub const ISLAND_M: f64 = 150.0;

/// Whether OpenMapTiles' `class` is one of the minor ways a past period leaves out where its ground did not stand.
pub fn minor(class: &str) -> bool {
    matches!(class, "minor" | "service" | "raceway")
}

/// What the period paved: today's every road, or only the arterials (the minor roads, service roads and tracks of
/// earth or gravel), or only the arterials within a town's paved streets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paving {
    Today,
    /// The minor roads, service roads and tracks unpaved.
    Arterials,
    /// Every road unpaved but within `radius` m of `at` (theatre metres), where the town's streets were paved.
    Town {
        at: [f64; 2],
        radius: f64,
    },
    /// Every road unpaved.
    None,
}

/// Japan's home islands, Iceland and the Solomons (south, west, north, east; degrees).
const JAPAN: [f64; 4] = [24.0, 122.5, 46.0, 146.0];
const ICELAND: [f64; 4] = [63.0, -25.0, 67.0, -13.0];
const SOLOMONS: [f64; 4] = [-12.5, 154.0, -4.5, 170.5];
/// Reykjavík's paved streets: within this of the old town's middle (latitude, longitude, m).
const REYKJAVIK: (f64, f64, f64) = (64.1466, -21.9426, 1500.0);

/// The paving of every period (to 1960) where (`lat`, `lon`) lies, in the frame about `frame`. Japan's minor roads
/// were earth or gravel (in 1970 some 15% of its general roads were paved; Tokyo's streets were half paved in 1937,
/// its arterials first); Iceland paved no road outside Reykjavík before the Reykjanesbraut (1965), and Reykjavík its
/// own streets from 1927; the Solomons' roads were coral and gravel. Elsewhere (Britain, the United States' coasts
/// and Hawaii) as today.
pub fn paving(lat: f64, lon: f64, frame: LatLon) -> Paving {
    let within = |b: [f64; 4]| (b[0]..=b[2]).contains(&lat) && (b[1]..=b[3]).contains(&lon);
    if within(JAPAN) {
        Paving::Arterials
    } else if within(ICELAND) {
        let (lat, lon, radius) = REYKJAVIK;
        Paving::Town {
            at: to_local(frame, LatLon { lat, lon }),
            radius,
        }
    } else if within(SOLOMONS) {
        Paving::None
    } else {
        Paving::Today
    }
}

/// A listed motorway section in the theatre's metres.
struct Route {
    points: Vec<[f64; 2]>,
    /// Drawn as a trunk road (it was an ordinary road in the period), its ramps not at all.
    road: bool,
}

impl Route {
    /// The distance from `p` to the route and the unit direction of its nearest segment.
    fn nearest(&self, p: [f64; 2]) -> (f64, [f64; 2]) {
        let mut best = (f64::INFINITY, [1.0, 0.0]);
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let len2 = d[0] * d[0] + d[1] * d[1];
            if len2 < 1e-6 {
                continue;
            }
            let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0);
            let q = [a[0] + t * d[0], a[1] + t * d[1]];
            let dist = (p[0] - q[0]).hypot(p[1] - q[1]);
            if dist < best.0 {
                let len = len2.sqrt();
                best = (dist, [d[0] / len, d[1] / len]);
            }
        }
        best
    }

    /// Whether a motorway's way at `p` heading `d` (a ramp: `ramp`) runs along the route.
    fn carries(&self, p: [f64; 2], d: [f64; 2], ramp: bool) -> bool {
        let (dist, along) = self.nearest(p);
        if ramp {
            dist <= RAMP_CORRIDOR_M
        } else {
            dist <= CORRIDOR_M
                && (along[0] * d[0] + along[1] * d[1]).abs() >= ALONG_DEG.to_radians().cos()
        }
    }
}

/// A run of a way as the period draws it: its points (theatre metres) and whether it is drawn as a trunk road (a
/// motorway that was an ordinary road then).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub points: Vec<[f64; 2]>,
    pub trunk: bool,
}

/// The period's rules for one page's ways: the period, the theatre's frame, whether the ground at a theatre point stood
/// (`period_land::stood`, or a test's), the motorways that stood, and the region's paving.
pub struct Roads<'a> {
    pub period: Period,
    frame: LatLon,
    stood: &'a (dyn Fn([f64; 2]) -> bool + Sync),
    routes: Vec<Route>,
    paving: Paving,
}

impl<'a> Roads<'a> {
    /// Today's roads: every way as it is.
    pub fn today() -> Roads<'static> {
        Roads {
            period: Period::Today,
            frame: LatLon { lat: 0.0, lon: 0.0 },
            stood: &|_| true,
            routes: Vec::new(),
            paving: Paving::Today,
        }
    }

    /// The roads of `period` about theatre point `at` (a page's middle) in the frame about `frame`, the ground's
    /// standing answered by `stood`.
    pub fn new(
        period: Period,
        frame: LatLon,
        at: [f64; 2],
        stood: &'a (dyn Fn([f64; 2]) -> bool + Sync),
    ) -> Self {
        let Some(year) = period.year() else {
            return Roads {
                period,
                frame,
                stood,
                routes: Vec::new(),
                paving: Paving::Today,
            };
        };
        let routes = MOTORWAYS
            .iter()
            .filter(|s| s.year <= year || s.road.is_some_and(|r| r <= year))
            .map(|s| Route {
                points: s
                    .route
                    .iter()
                    .map(|&(lat, lon)| to_local(frame, LatLon { lat, lon }))
                    .collect(),
                road: s.year > year,
            })
            .collect();
        let here = to_lat_lon(frame, at);
        Roads {
            period,
            frame,
            stood,
            routes,
            paving: paving(here.lat, here.lon, frame),
        }
    }

    pub fn is_today(&self) -> bool {
        !self.period.is_past()
    }

    /// Whether the ground at theatre point `p` stood in the period (`period_land::stood`; always today).
    pub fn stood(&self, p: [f64; 2]) -> bool {
        (self.stood)(p)
    }

    /// How way `t` is drawn in the period (as [`super::style_of`]), none where the period has none of its kind.
    pub fn style<P>(&self, t: &Transportation<P>) -> Option<(StrokeKind, f32, f32)> {
        let style = super::style_of(t)?;
        if self.is_today() {
            return Some(style);
        }
        if t.class == "busway" {
            return None;
        }
        Some(style)
    }

    /// How a run of way `t` is drawn: its style ([`Self::style`]'s `style`, or a trunk road's for a [`Run::trunk`]),
    /// its kind paved as the period paved it there (judged at its first segment's middle: a run of
    /// [`Self::paving_pieces`] is paved or not throughout).
    pub fn run_style<P>(
        &self,
        t: &Transportation<P>,
        style: (StrokeKind, f32, f32),
        run: &Run,
    ) -> (StrokeKind, f32, f32) {
        let (class, (kind, half, tone)) = if run.trunk {
            ("trunk", super::trunk_style())
        } else {
            (t.class.as_str(), style)
        };
        if self.paving == Paving::Today {
            return (kind, half, tone);
        }
        let (a, b) = (run.points[0], run.points[1.min(run.points.len() - 1)]);
        let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        (self.paved(class, kind, mid), half, tone)
    }

    /// `run` cut where the period's paving changes along it (a town's paved streets' edge, found exactly), so each
    /// piece is paved or not throughout, the same pieces whichever way the line runs; the run whole elsewhere.
    pub fn paving_pieces(&self, run: Run) -> Vec<Run> {
        let Paving::Town { at, radius } = self.paving else {
            return vec![run];
        };
        split_at_circle(&run.points, at, radius)
            .into_iter()
            .map(|points| Run {
                points,
                trunk: run.trunk,
            })
            .collect()
    }

    /// The paving of a road of `class` drawn as `kind` at theatre point `p`.
    pub fn paved(&self, class: &str, kind: StrokeKind, p: [f64; 2]) -> StrokeKind {
        if kind != StrokeKind::Paved {
            return kind;
        }
        let arterial = matches!(
            class,
            "motorway" | "trunk" | "primary" | "secondary" | "tertiary"
        );
        let unpaved = match self.paving {
            Paving::Today => false,
            Paving::Arterials => !arterial,
            Paving::Town { at, radius } => (p[0] - at[0]).hypot(p[1] - at[1]) > radius,
            Paving::None => true,
        };
        if unpaved { StrokeKind::Unpaved } else { kind }
    }

    /// Whether a bridge's way (its points, theatre metres) stood in the period: one built after it is not drawn
    /// (`naval_terrain::infra::period::bridge_stood`; its sea span `sea_m`, the longest run of it over the sea).
    pub fn bridge_stood(&self, points: &[[f64; 2]], sea_m: f64) -> bool {
        let Some(year) = self.period.year() else {
            return true;
        };
        let (Some(a), Some(b)) = (points.first(), points.last()) else {
            return true;
        };
        let at = to_lat_lon(self.frame, [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
        infra_period::bridge_stood(year, at.lat, at.lon, sea_m)
    }

    /// The runs of way `t`'s `line` drawn in the period (theatre metres): the line whole, part of it or none.
    /// `natural`: whether each end of the line is the way's own (not cut at its tile's edge).
    pub fn runs<P>(
        &self,
        t: &Transportation<P>,
        line: &[[f64; 2]],
        natural: [bool; 2],
    ) -> Vec<Run> {
        let tag = |trunk: bool| move |points| Run { points, trunk };
        if self.is_today() {
            return vec![tag(false)(line.to_vec())];
        }
        if t.class == "motorway" {
            if self.routes.is_empty() {
                return Vec::new();
            }
            let ramp = t.ramp;
            let motorway = |p, d| self.routes.iter().any(|r| !r.road && r.carries(p, d, ramp));
            let mut out: Vec<Run> = runs(line, natural, motorway)
                .into_iter()
                .map(tag(false))
                .collect();
            if !ramp && self.routes.iter().any(|r| r.road) {
                let road = |p, d| {
                    !motorway(p, d) && self.routes.iter().any(|r| r.road && r.carries(p, d, false))
                };
                out.extend(runs(line, natural, road).into_iter().map(tag(true)));
            }
            return out;
        }
        if minor(&t.class) {
            return runs(line, natural, |p, _| (self.stood)(p))
                .into_iter()
                .map(tag(false))
                .collect();
        }
        vec![tag(false)(line.to_vec())]
    }
}

/// `points` cut where they cross the circle of `radius` about `at` (each crossing found exactly and shared by the pieces
/// either side), so each piece lies wholly within it or outside.
pub fn split_at_circle(points: &[[f64; 2]], at: [f64; 2], radius: f64) -> Vec<Vec<[f64; 2]>> {
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut piece = vec![first];
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let f = [a[0] - at[0], a[1] - at[1]];
        let qa = d[0] * d[0] + d[1] * d[1];
        let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
        let qc = f[0] * f[0] + f[1] * f[1] - radius * radius;
        let disc = qb * qb - 4.0 * qa * qc;
        if qa > 1e-12 && disc > 0.0 {
            let root = disc.sqrt();
            for t in [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)] {
                if t > 1e-9 && t < 1.0 - 1e-9 {
                    let p = [a[0] + t * d[0], a[1] + t * d[1]];
                    piece.push(p);
                    out.push(std::mem::replace(&mut piece, vec![p]));
                }
            }
        }
        piece.push(b);
    }
    out.push(piece);
    out
}

/// The longest run of `line` over the sea (m), by `sea` (sampled every 8 m).
pub fn sea_run(line: &[[f64; 2]], sea: impl Fn([f64; 2]) -> bool) -> f64 {
    let mut best = 0.0f64;
    let mut run = 0.0f64;
    let mut last: Option<[f64; 2]> = None;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let n = (len / 8.0).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f64 / n as f64;
            let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            let step = last.map_or(0.0, |q| (p[0] - q[0]).hypot(p[1] - q[1]));
            if sea(p) {
                run += step;
                best = best.max(run);
            } else {
                run = 0.0;
            }
            last = Some(p);
        }
    }
    best
}

/// The runs of `line` where `keep(point, unit direction)` holds, cut to [`CUT_M`], the gaps under [`GAP_M`] closed, and
/// the runs shorter than [`STUB_M`] at a cut ([`ISLAND_M`] at two) dropped; an end of the line not `natural` (cut at
/// its tile's edge) counts as a cut. The line whole when it is kept whole.
pub fn runs(
    line: &[[f64; 2]],
    natural: [bool; 2],
    keep: impl Fn([f64; 2], [f64; 2]) -> bool,
) -> Vec<Vec<[f64; 2]>> {
    // Arc lengths of the vertices.
    let mut along = Vec::with_capacity(line.len());
    let mut s = 0.0;
    along.push(0.0);
    for w in line.windows(2) {
        s += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        along.push(s);
    }
    let length = s;
    if line.len() < 2 || length < 1e-6 {
        return if line.len() >= 2 && keep(line[0], [1.0, 0.0]) {
            vec![line.to_vec()]
        } else {
            Vec::new()
        };
    }
    // The point and direction at arc length `s`.
    let at = |s: f64| -> ([f64; 2], [f64; 2]) {
        let k = along.partition_point(|x| *x <= s).clamp(1, line.len() - 1);
        let (a, b) = (line[k - 1], line[k]);
        let len = along[k] - along[k - 1];
        let t = if len > 0.0 {
            ((s - along[k - 1]) / len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = if len > 0.0 {
            [(b[0] - a[0]) / len, (b[1] - a[1]) / len]
        } else {
            [1.0, 0.0]
        };
        ([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])], d)
    };
    let test = |s: f64| {
        let (p, d) = at(s);
        keep(p, d)
    };
    // Samples every STEP_M or less, and the kept intervals between them (each change found by bisection).
    let n = (length / STEP_M).ceil().max(1.0) as usize;
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    let mut prev_s = 0.0;
    let mut prev = test(0.0);
    let mut open = prev.then_some(0.0);
    for k in 1..=n {
        let s = length * k as f64 / n as f64;
        let here = test(s);
        if here != prev {
            let (mut lo, mut hi) = (prev_s, s);
            while hi - lo > CUT_M {
                let mid = 0.5 * (lo + hi);
                if test(mid) == prev {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let cut = 0.5 * (lo + hi);
            if here {
                open = Some(cut);
            } else if let Some(start) = open.take() {
                intervals.push((start, cut));
            }
        }
        prev = here;
        prev_s = s;
    }
    if let Some(start) = open {
        intervals.push((start, length));
    }
    if intervals.len() == 1 && intervals[0] == (0.0, length) {
        return vec![line.to_vec()];
    }
    // Close the short gaps.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in intervals {
        match merged.last_mut() {
            Some(last) if a - last.1 < GAP_M => last.1 = b,
            _ => merged.push((a, b)),
        }
    }
    if merged.len() == 1 && merged[0] == (0.0, length) {
        return vec![line.to_vec()];
    }
    // Drop the stubs: an end at a cut is any end but the line's own natural ones.
    let cut_at =
        |s: f64, end: usize| !(natural[end] && (if end == 0 { s <= 0.0 } else { s >= length }));
    merged.retain(|&(a, b)| {
        let cuts = usize::from(cut_at(a, 0)) + usize::from(cut_at(b, 1));
        let least = match cuts {
            0 => 0.0,
            1 => STUB_M,
            _ => ISLAND_M,
        };
        b - a >= least
    });
    merged
        .into_iter()
        .map(|(a, b)| {
            let mut out = vec![at(a).0];
            for (k, &s) in along.iter().enumerate() {
                if s > a && s < b {
                    out.push(line[k]);
                }
            }
            out.push(at(b).0);
            out
        })
        .filter(|r| r.len() >= 2)
        .collect()
}

/// One tile's ways under a period's [`Roads`]: what each way's lines are drawn as.
pub struct TileRoads<'r, 't> {
    roads: &'r Roads<'r>,
    /// The tile proper's corners (a line's end inside it is the way's own), past periods only.
    quad: Option<[[f64; 2]; 4]>,
    /// The tile's sea (ocean and dock areas within the rectangle), gathered at its first bridge.
    sea: std::cell::OnceCell<Vec<&'t naval_osm::Polygon<[f64; 2]>>>,
    tile: &'t OsmTile,
    min: [f64; 2],
    max: [f64; 2],
}

impl<'r, 't> TileRoads<'r, 't> {
    /// `tile`'s ways within the rectangle `min` to `max` (theatre metres) under `roads`.
    pub fn new(roads: &'r Roads<'r>, tile: &'t OsmTile, min: [f64; 2], max: [f64; 2]) -> Self {
        Self {
            roads,
            quad: (!roads.is_today()).then(|| tile.quad(roads.frame)),
            sea: std::cell::OnceCell::new(),
            tile,
            min,
            max,
        }
    }

    fn sea(&self, p: [f64; 2]) -> bool {
        let polys = self.sea.get_or_init(|| {
            self.tile
                .water_in(self.min, self.max)
                .filter(|w| matches!(w.class.as_str(), "ocean" | "dock"))
                .filter_map(|w| match &w.geometry {
                    Geometry::Polygons(ps) => Some(ps.iter()),
                    _ => None,
                })
                .flatten()
                .collect()
        });
        polys.iter().any(|poly| {
            let mut inside = false;
            for ring in std::iter::once(&poly.exterior).chain(poly.holes.iter()) {
                let n = ring.len();
                for k in 0..n {
                    let (a, b) = (ring[k], ring[(k + 1) % n]);
                    if (a[1] > p[1]) != (b[1] > p[1]) {
                        let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                        if p[0] < x {
                            inside = !inside;
                        }
                    }
                }
            }
            inside
        })
    }

    /// Way `t`'s `lines` as the period draws them, each with its style (`style` today's, [`Roads::run_style`]'s in
    /// the period) and whether it is drawn as a trunk road: the lines whole today; in a past period none of a bridge
    /// built after it, the runs [`Roads::runs`] keeps of the rest.
    pub fn lines(
        &self,
        t: &Transportation<[f64; 2]>,
        style: (StrokeKind, f32, f32),
        lines: &[Vec<[f64; 2]>],
    ) -> Vec<((StrokeKind, f32, f32), Run)> {
        let Some(quad) = &self.quad else {
            return lines
                .iter()
                .map(|l| {
                    (
                        style,
                        Run {
                            points: l.clone(),
                            trunk: false,
                        },
                    )
                })
                .collect();
        };
        let mut out = Vec::new();
        for line in lines {
            if line.len() < 2 {
                continue;
            }
            if t.brunnel == Brunnel::Bridge
                && !self
                    .roads
                    .bridge_stood(line, sea_run(line, |p| self.sea(p)))
            {
                continue;
            }
            let natural = [in_quad(quad, line[0]), in_quad(quad, line[line.len() - 1])];
            for run in self.roads.runs(t, line, natural) {
                for piece in self.roads.paving_pieces(run) {
                    out.push((self.roads.run_style(t, style, &piece), piece));
                }
            }
        }
        out
    }
}

/// Whether `p` lies in the tile proper whose corners are `quad` (north-west, north-east, south-east, south-west): its
/// north and west edges in, its south and east out, so a point on an edge two tiles share is one tile's.
pub fn in_quad(quad: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    (0..4).all(|k| {
        let (a, b) = (quad[k], quad[(k + 1) % 4]);
        let e = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if k == 0 || k == 3 { e >= 0.0 } else { e > 0.0 }
    })
}

/// `hash` (a page's strokes', [`strokes_hash`]) with its lamps folded in: a page whose floods changed with the period's
/// ground while its strokes did not is a change too.
pub fn lamps_hash(hash: u64, lamps: &[naval_terrain::night::Lamp]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash.hash(&mut h);
    for l in lamps {
        l.at[0].to_bits().hash(&mut h);
        l.at[1].to_bits().hash(&mut h);
        l.height.to_bits().hash(&mut h);
        l.info.hash(&mut h);
    }
    h.finish()
}

/// A hash of strokes (their kinds, widths, tones and points), to tell a page made again the same from one changed.
pub fn strokes_hash(strokes: &[naval_terrain::Stroke]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for s in strokes {
        (s.kind as u8).hash(&mut h);
        s.half_width.to_bits().hash(&mut h);
        s.tone.to_bits().hash(&mut h);
        for p in &s.points {
            p[0].to_bits().hash(&mut h);
            p[1].to_bits().hash(&mut h);
        }
        u8::MAX.hash(&mut h);
    }
    h.finish()
}
