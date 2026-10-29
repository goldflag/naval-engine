//! Where a generated base's runway lies ([`fit`]): today's runway near its point where the map has one (cut to the
//! period's length), else the flattest dry ground long enough within a few kilometres of its point, off the water and
//! steep slopes, nearer its own point than any other base's. Metres about the base's point (x east, y south, the
//! theatre's x and z); the ground and the water are the host's.

use glam::Vec2;

use super::SizeClass;

/// The least height the base's ground may have (m above the sea): off the beach and the swamp at its back.
pub const DRY_M: f32 = 2.0;
/// The least height a temperate base's ground may have: a polder behind its dikes (Friesland's, North Holland's) lies
/// below the sea; the map's water keeps it off the sea.
pub const LOWLAND_M: f32 = -12.0;
/// The steepest grade along the runway, and across it.
const MAX_GRADE: f32 = 0.03;
const MAX_CROSS: f32 = 0.05;
/// The most the ground may stand off a straight line along the runway (m, its root mean square).
const MAX_ROUGH_M: f32 = 3.0;
/// How far past each end and each side of the paving the ground must be dry and fair (m).
const MARGIN_M: f32 = 50.0;
/// How much nearer its own point than another base's every point of the runway must lie (m).
const OWN_M: f32 = 60.0;
/// Today's runway is taken only if it is at least this long (m).
const LEAST_RUNWAY_M: f32 = 700.0;

/// What a base's site is fitted from (metres about its point).
pub struct SiteInput<'a> {
    /// The base's seed ([`super::BaseEntry::seed`]): its ties broken alike every visit.
    pub seed: u32,
    pub size: SizeClass,
    /// The template laying it out (its runway's size: [`super::Template::runway`]).
    pub template: super::Template,
    /// Whether its runways are gravel (its catalog entry's `surface`).
    pub gravel: bool,
    /// The runway's heading the catalog hints (degrees clockwise from true north).
    pub heading_hint: Option<f32>,
    /// The ground (m above the sea).
    pub ground: &'a dyn Fn(Vec2) -> f32,
    /// Whether a point is water (the sea, a river, a lake).
    pub water: &'a dyn Fn(Vec2) -> bool,
    /// The other bases' points about it (theirs, and the famous fields'): its runway keeps nearer its own.
    pub others: &'a [Vec2],
    /// Today's runway near its point, its two ends: the base is laid along it.
    pub runway: Option<[Vec2; 2]>,
}

/// A base's runway: its middle, its direction (unit, from its first end to its second), its length, its paved half
/// width and shoulders (m), the side its taxiway and dispersal lie on (+1 the runway's right, −1 its left: the side with
/// more dry land), and whether it lies along today's runway.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    pub centre: Vec2,
    pub dir: Vec2,
    pub length: f32,
    pub half: f32,
    pub shoulder: f32,
    pub side: f32,
    pub aligned: bool,
}

impl Site {
    /// The runway's right: its direction turned a quarter clockwise seen from above (x east, y south).
    pub fn right(&self) -> Vec2 {
        Vec2::new(-self.dir.y, self.dir.x)
    }

    /// The point `s` metres along from the middle and `t` across (toward the right).
    pub fn at(&self, s: f32, t: f32) -> Vec2 {
        self.centre + self.dir * s + self.right() * t
    }

    pub fn ends(&self) -> [Vec2; 2] {
        [
            self.at(-0.5 * self.length, 0.0),
            self.at(0.5 * self.length, 0.0),
        ]
    }

    /// The heading from its first end to its second (degrees clockwise from true north).
    pub fn heading(&self) -> f32 {
        self.dir.x.atan2(-self.dir.y).to_degrees().rem_euclid(360.0)
    }
}

/// The direction of a heading (degrees clockwise from true north; x east, y south).
pub fn direction(heading: f32) -> Vec2 {
    let r = heading.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// The base's runway, or none where no dry, fair ground long enough lies within its search.
pub fn fit(input: &SiteInput<'_>) -> Option<Site> {
    let size = input.size;
    let (length, half, shoulder) = input.template.runway(size);
    if let Some([a, b]) = input.runway {
        let along = a.distance(b);
        if along >= LEAST_RUNWAY_M {
            let dir = (b - a) / along;
            let centre = (a + b) * 0.5;
            let length = along.min(length);
            let side = sides(input, centre, dir, length, half + shoulder).map_or(1.0, |s| s.0);
            return Some(Site {
                centre,
                dir,
                length,
                half,
                shoulder,
                side,
                aligned: true,
            });
        }
    }
    // A coarse search: centres 125 m apart within the search, headings 15° apart.
    let search = size.search_m();
    let step = 125.0;
    let n = (search / step).ceil() as i32;
    let mut best: Vec<(f32, Vec2, f32)> = Vec::new();
    for j in -n..=n {
        for i in -n..=n {
            let centre = Vec2::new(i as f32, j as f32) * step;
            if centre.length() > search {
                continue;
            }
            for k in 0..12 {
                let heading = k as f32 * 15.0;
                if let Some(cost) = score(input, centre, heading, length, half + shoulder) {
                    best.push((cost, centre, heading));
                }
            }
        }
    }
    best.sort_by(|a, b| a.0.total_cmp(&b.0));
    best.truncate(4);
    // Each of the best few refined: 25 m and 2.5° about it.
    let mut found: Option<(f32, Vec2, f32)> = None;
    for &(cost, centre, heading) in &best {
        let mut here = (cost, centre, heading);
        for dj in -3..=3 {
            for di in -3..=3 {
                for dk in -4..=4 {
                    let c = centre + Vec2::new(di as f32, dj as f32) * 25.0;
                    let h = heading + dk as f32 * 2.5;
                    if c.length() > search {
                        continue;
                    }
                    if let Some(cost) = score(input, c, h, length, half + shoulder)
                        && cost < here.0
                    {
                        here = (cost, c, h);
                    }
                }
            }
        }
        if found.is_none_or(|f| here.0 < f.0) {
            found = Some(here);
        }
    }
    let (_, centre, heading) = found?;
    let dir = direction(heading);
    let side = sides(input, centre, dir, length, half + shoulder).map_or(1.0, |s| s.0);
    Some(Site {
        centre,
        dir,
        length,
        half,
        shoulder,
        side,
        aligned: false,
    })
}

/// Whether the base may stand at `p`: dry, high enough and nearer its own point than another base's.
pub fn usable(input: &SiteInput<'_>, p: Vec2) -> bool {
    let least = if input.template.temperate() {
        LOWLAND_M
    } else {
        DRY_M
    };
    if (input.water)(p) || (input.ground)(p) < least {
        return false;
    }
    let own = p.length();
    input.others.iter().all(|o| own + OWN_M < p.distance(*o))
}

/// The cost of a runway with its middle at `centre` along `heading` (degrees), `reach` its paving's half width with the
/// shoulders: none where any of it is wet or too steep. Lower is better: the ground's roughness along it, its grade and
/// cross slope, the land beside it, its distance from the base's point, the heading's from the hint, and a tie-break
/// from the seed.
fn score(
    input: &SiteInput<'_>,
    centre: Vec2,
    heading: f32,
    length: f32,
    reach: f32,
) -> Option<f32> {
    let dir = direction(heading);
    let right = Vec2::new(-dir.y, dir.x);
    let half_run = 0.5 * length + MARGIN_M;
    let across = reach + MARGIN_M;
    let samples = (2.0 * half_run / 40.0).ceil() as usize;
    // The middle line first: most candidates fail on it.
    let mut line = Vec::with_capacity(samples + 1);
    for k in 0..=samples {
        let s = -half_run + 2.0 * half_run * k as f32 / samples as f32;
        let p = centre + dir * s;
        if !usable(input, p) {
            return None;
        }
        line.push((s, (input.ground)(p)));
    }
    let mut cross = 0.0f32;
    for (k, &(s, _)) in line.iter().enumerate() {
        if k % 2 == 1 && k != samples {
            continue;
        }
        let a = centre + dir * s + right * across;
        let b = centre + dir * s - right * across;
        if !usable(input, a) || !usable(input, b) {
            return None;
        }
        cross = cross.max(((input.ground)(a) - (input.ground)(b)).abs() / (2.0 * across));
    }
    if cross > MAX_CROSS {
        return None;
    }
    // A straight line through the heights along it: its grade and how far the ground stands off it.
    let n = line.len() as f32;
    let (ms, mh) = line
        .iter()
        .fold((0.0, 0.0), |(a, b), (s, h)| (a + s / n, b + h / n));
    let (mut sxx, mut sxy) = (0.0f32, 0.0f32);
    for (s, h) in &line {
        sxx += (s - ms) * (s - ms);
        sxy += (s - ms) * (h - mh);
    }
    let grade = sxy / sxx.max(1e-3);
    if grade.abs() > MAX_GRADE {
        return None;
    }
    let rough = (line
        .iter()
        .map(|(s, h)| (h - (mh + grade * (s - ms))).powi(2))
        .sum::<f32>()
        / n)
        .sqrt();
    if rough > MAX_ROUGH_M {
        return None;
    }
    let (wide, narrow) = sides_at(input, centre, dir, length, reach);
    let hint = input.heading_hint.map_or(0.0, |h| {
        let d = (heading - h).rem_euclid(180.0);
        d.min(180.0 - d) / 30.0
    });
    let tie = crate::buildings::hash01(
        (centre.x / 25.0).round() as i64,
        (centre.y / 25.0).round() as i64,
        i64::from(input.seed) ^ (heading * 4.0).round() as i64,
    );
    Some(
        rough
            + grade.abs() * 60.0
            + cross * 30.0
            + (1.0 - wide) * 3.0
            + (1.0 - narrow) * 1.5
            + centre.length() / 1500.0
            + hint
            + tie * 0.05,
    )
}

/// The share of dry ground beside a runway on its right and its left (150 and 300 m past its shoulders, along 90% of
/// it).
fn dry_beside(input: &SiteInput<'_>, centre: Vec2, dir: Vec2, length: f32, reach: f32) -> [f32; 2] {
    let right = Vec2::new(-dir.y, dir.x);
    let mut dry = [0u32; 2];
    for k in 0..=8 {
        let s = -0.45 * length + 0.9 * length * k as f32 / 8.0;
        for (i, sign) in [1.0f32, -1.0].into_iter().enumerate() {
            for t in [150.0, 300.0] {
                if usable(input, centre + dir * s + right * (sign * (reach + t))) {
                    dry[i] += 1;
                }
            }
        }
    }
    dry.map(|d| d as f32 / 18.0)
}

/// The share of dry ground beside a runway on its better side and its worse.
fn sides_at(input: &SiteInput<'_>, centre: Vec2, dir: Vec2, length: f32, reach: f32) -> (f32, f32) {
    let [r, l] = dry_beside(input, centre, dir, length, reach);
    (r.max(l), r.min(l))
}

/// The side with more dry land beside the runway (+1 its right), with that side's share; none where neither has any.
fn sides(
    input: &SiteInput<'_>,
    centre: Vec2,
    dir: Vec2,
    length: f32,
    reach: f32,
) -> Option<(f32, f32)> {
    let [r, l] = dry_beside(input, centre, dir, length, reach);
    if r == 0.0 && l == 0.0 {
        return None;
    }
    Some(if r >= l { (1.0, r) } else { (-1.0, l) })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A coast running north and south at x = −400 (the sea west of it), a plain to x = 1,200 rising 1 m in 200, then
    /// hills; a creek across the plain at y = 1,500.
    pub fn coast(p: Vec2) -> f32 {
        if p.x < -400.0 {
            -6.0
        } else if p.x < 1200.0 {
            4.0 + (p.x + 400.0) / 200.0 + 0.6 * (p.y * 0.01).sin()
        } else {
            12.0 + (p.x - 1200.0) * 0.25
        }
    }

    pub fn creek(p: Vec2) -> bool {
        p.x < -400.0 || (p.y - 1500.0).abs() < 15.0
    }

    #[test]
    fn a_runway_lies_on_dry_fair_ground_and_the_same_every_time() {
        let input = SiteInput {
            seed: 77,
            size: SizeClass::BomberField,
            template: super::super::Template::UsPacificStrip,
            gravel: false,
            heading_hint: None,
            ground: &coast,
            water: &creek,
            others: &[],
            runway: None,
        };
        let site = fit(&input).expect("a site");
        assert_eq!(fit(&input), Some(site), "fitted alike");
        assert!(!site.aligned);
        // Along the coast (north-south), as the plain runs.
        let h = site.heading().rem_euclid(180.0);
        assert!(!(30.0..=150.0).contains(&h), "heading {h}");
        // Every point of its paving and shoulders dry, fair and off the hills.
        let reach = site.half + site.shoulder;
        for k in 0..=40 {
            let s = -0.5 * site.length + site.length * k as f32 / 40.0;
            for t in [-reach, 0.0, reach] {
                let p = site.at(s, t);
                assert!(!creek(p) && coast(p) >= DRY_M, "{p} wet or low");
                assert!(p.x < 1200.0, "{p} in the hills");
            }
        }
        let [a, b] = site.ends();
        assert!((coast(a) - coast(b)).abs() / site.length <= MAX_GRADE);
        // Its taxiway's side is dry land.
        for t in [150.0, 300.0] {
            let beside = site.at(0.0, site.side * (reach + t));
            assert!(usable(&input, beside), "{beside}");
        }
    }

    #[test]
    fn a_runway_keeps_nearer_its_own_point_and_follows_todays() {
        // Another base 900 m east: every point of this one's runway nearer its own point.
        let others = [Vec2::new(900.0, 0.0)];
        let input = SiteInput {
            seed: 5,
            size: SizeClass::FighterStrip,
            template: super::super::Template::UsPacificStrip,
            gravel: false,
            heading_hint: Some(0.0),
            ground: &coast,
            water: &creek,
            others: &others,
            runway: None,
        };
        let site = fit(&input).expect("a site");
        for k in 0..=20 {
            let p = site.at(-0.5 * site.length + site.length * k as f32 / 20.0, 0.0);
            assert!(p.length() < p.distance(others[0]), "{p}");
        }
        // Today's runway of 2,600 m: laid along it, cut to the strip's length about its middle.
        let a = Vec2::new(100.0, -1300.0);
        let b = Vec2::new(300.0, 1300.0);
        let along = SiteInput {
            runway: Some([a, b]),
            ..input
        };
        let site = fit(&along).expect("aligned");
        assert!(site.aligned);
        assert_eq!(site.length, SizeClass::FighterStrip.length());
        assert!(site.centre.distance((a + b) * 0.5) < 1e-3);
        assert!(site.dir.dot((b - a).normalize()) > 0.9999);
    }

    #[test]
    fn no_site_where_all_is_water() {
        let input = SiteInput {
            seed: 1,
            size: SizeClass::FighterStrip,
            template: super::super::Template::UsPacificStrip,
            gravel: false,
            heading_hint: None,
            ground: &|_| -10.0,
            water: &|_| true,
            others: &[],
            runway: None,
        };
        assert_eq!(fit(&input), None);
    }

    #[test]
    fn each_template_is_fitted_to_its_own_runway() {
        use super::super::Template;
        for (template, size, length, width) in [
            (
                Template::IjnSolomonsField,
                SizeClass::FighterStrip,
                1100.0,
                56.0,
            ),
            (
                Template::IjnSolomonsField,
                SizeClass::BomberField,
                1350.0,
                60.0,
            ),
            (Template::RafStation, SizeClass::BomberField, 1830.0, 46.0),
            (Template::RafStation, SizeClass::FighterStrip, 1100.0, 90.0),
            (
                Template::LuftwaffeField,
                SizeClass::BomberField,
                1500.0,
                60.0,
            ),
            (
                Template::LuftwaffeField,
                SizeClass::FighterStrip,
                1000.0,
                80.0,
            ),
        ] {
            let input = SiteInput {
                seed: 12,
                size,
                template,
                gravel: false,
                heading_hint: None,
                ground: &coast,
                water: &creek,
                others: &[],
                runway: None,
            };
            let site = fit(&input).expect("a site");
            assert_eq!(site.length, length);
            assert_eq!(2.0 * (site.half + site.shoulder), width);
            assert_eq!(fit(&input), Some(site), "the same every time");
            // Along today's runway (Munda's, Buka's, Leeuwarden's), cut to its length about its middle.
            let (a, b) = (Vec2::new(-200.0, -1050.0), Vec2::new(-100.0, 1050.0));
            let along = fit(&SiteInput {
                runway: Some([a, b]),
                ..input
            })
            .expect("aligned");
            assert!(along.aligned && along.length == length);
            assert!(along.centre.distance((a + b) * 0.5) < 1e-3);
        }
    }
}
