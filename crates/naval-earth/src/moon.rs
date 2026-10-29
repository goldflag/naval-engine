//! The moon over the theatre: where it stands and how much of it is lit, for the native client's night sky. The
//! Astronomical Almanac's low-precision series (a third of a degree in longitude, a fifth in latitude over the
//! twentieth century), seen from the place itself: the moon is near enough that it stands up to a degree lower than
//! from the Earth's centre. Nothing in the simulation reads it: sighting takes its dark from the sun alone
//! (`sighting.rs`).
use super::sun::{days, ecliptic_longitude, sidereal};

/// Where the moon stands over a point of the theatre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonPosition {
    /// Degrees above the horizon, as seen from the place.
    pub elevation_deg: f64,
    /// True bearing, degrees clockwise from north.
    pub azimuth_deg: f64,
    /// Its age as a share of the month: 0 new, 0.25 the first quarter, 0.5 full, 0.75 the last quarter.
    pub phase: f64,
}

/// The moon at `lat, lon` (degrees), `seconds` after the scenario's local `start`; none for a start that does not
/// parse.
pub fn position(
    start: &str,
    utc_offset_hours: f64,
    seconds: f64,
    lat: f64,
    lon: f64,
) -> Option<MoonPosition> {
    let n = days(start, utc_offset_hours, seconds)?;
    let t = n / 36525.0;
    let rad = f64::to_radians;
    let sin = |a: f64, b: f64| rad(a + b * t).sin();
    let cos = |a: f64, b: f64| rad(a + b * t).cos();
    let longitude = rad(218.32 + 481_267.881 * t + 6.29 * sin(135.0, 477_198.87)
        - 1.27 * sin(259.3, -413_335.36)
        + 0.66 * sin(235.7, 890_534.22)
        + 0.21 * sin(269.9, 954_397.74)
        - 0.19 * sin(357.5, 35_999.05)
        - 0.11 * sin(186.5, 966_404.03));
    let latitude = rad(5.13 * sin(93.3, 483_202.02) + 0.28 * sin(228.2, 960_400.89)
        - 0.28 * sin(318.3, 6_003.15)
        - 0.17 * sin(217.6, -407_332.21));
    // Its horizontal parallax: the Earth's radius as seen from the moon.
    let parallax = rad(0.9508
        + 0.0518 * cos(135.0, 477_198.87)
        + 0.0095 * cos(259.3, -413_335.36)
        + 0.0078 * cos(235.7, 890_534.22)
        + 0.0028 * cos(269.9, 954_397.74));
    // From the ecliptic to the equator, then to the place's horizon by the hour angle, as the sun's.
    let obliquity = rad(23.439 - 0.000_000_4 * n);
    let (x, along) = (
        latitude.cos() * longitude.cos(),
        latitude.cos() * longitude.sin(),
    );
    let y = obliquity.cos() * along - obliquity.sin() * latitude.sin();
    let z = obliquity.sin() * along + obliquity.cos() * latitude.sin();
    let (right_ascension, declination) = (y.atan2(x), z.clamp(-1.0, 1.0).asin());
    let hour_angle = rad(sidereal(n, lon)) - right_ascension;
    let phi = rad(lat);
    // East, north and up of the place, in Earth radii: the moon from the centre, less the place itself.
    let distance = 1.0 / parallax.sin();
    let east = -declination.cos() * hour_angle.sin() * distance;
    let north = (declination.sin() * phi.cos() - declination.cos() * hour_angle.cos() * phi.sin())
        * distance;
    let up = (declination.sin() * phi.sin() + declination.cos() * hour_angle.cos() * phi.cos())
        * distance
        - 1.0;
    Some(MoonPosition {
        elevation_deg: up.atan2(east.hypot(north)).to_degrees(),
        azimuth_deg: east.atan2(north).to_degrees().rem_euclid(360.0),
        phase: ((longitude - ecliptic_longitude(n)) / std::f64::consts::TAU).rem_euclid(1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sun;

    /// Degrees between two places in the sky given as (elevation, true bearing).
    fn apart(a: (f64, f64), b: (f64, f64)) -> f64 {
        let unit = |(elevation, bearing): (f64, f64)| {
            let (e, b) = (elevation.to_radians(), bearing.to_radians());
            [e.cos() * b.sin(), e.cos() * b.cos(), e.sin()]
        };
        let (a, b) = (unit(a), unit(b));
        (a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    #[test]
    fn the_moon_covers_the_sun_at_the_eclipse_of_august_1999() {
        // Greatest eclipse, 11:03 UTC on 11 August 1999 at 45.1°N 24.3°E (Romania): total there, so the moon seen
        // from the place stands on the sun. From the Earth's centre it stood half a degree off.
        let (lat, lon) = (45.1, 24.3);
        let moon = position("1999-08-11T11:03", 0.0, 0.0, lat, lon).unwrap();
        let sun = sun::position("1999-08-11T11:03", 0.0, 0.0, lat, lon);
        let gap = apart(
            (moon.elevation_deg, moon.azimuth_deg),
            (sun.elevation_deg, sun.azimuth_deg),
        );
        assert!(gap < 0.6, "{gap} degrees from the sun: {moon:?} {sun:?}");
        assert!(moon.phase < 0.01 || moon.phase > 0.99, "{moon:?}");
    }

    #[test]
    fn the_moon_stands_opposite_the_sun_at_the_eclipse_of_january_2000() {
        // Mid totality of the lunar eclipse, 04:44 UTC on 21 January 2000, from the Caribbean where it was high.
        let (lat, lon) = (20.0, -70.0);
        let moon = position("2000-01-21T04:44", 0.0, 0.0, lat, lon).unwrap();
        let sun = sun::position("2000-01-21T04:44", 0.0, 0.0, lat, lon);
        let gap = apart(
            (moon.elevation_deg, moon.azimuth_deg),
            (-sun.elevation_deg, sun.azimuth_deg + 180.0),
        );
        // Within the parallax of a moon that high (a third of a degree) and the series' error.
        assert!(
            gap < 0.9,
            "{gap} degrees from the anti-sun: {moon:?} {sun:?}"
        );
        assert!((moon.phase - 0.5).abs() < 0.005, "{moon:?}");
        assert!(moon.elevation_deg > 50.0, "{moon:?}");
    }

    #[test]
    fn the_night_of_savo_island_had_no_moon_until_the_small_hours() {
        // 8 to 9 August 1942 off Savo (9.1°S 159.8°E, UTC+11): the new moon fell on the 12th, so a thin waning
        // crescent rose a few hours before the sun. Mikawa's cruisers attacked at 01:40 under no moon.
        let at = |start: &str| position(start, 11.0, 0.0, -9.1, 159.8).unwrap();
        let attack = at("1942-08-09T01:40");
        assert!(attack.elevation_deg < 0.0, "{attack:?}");
        assert!((0.84..0.92).contains(&attack.phase), "{attack:?}");
        // Before dawn it is up in the east.
        let dawn = at("1942-08-09T05:30");
        assert!(dawn.elevation_deg > 10.0, "{dawn:?}");
        assert!((40.0..110.0).contains(&dawn.azimuth_deg), "{dawn:?}");
    }

    #[test]
    fn the_phase_runs_through_a_month_and_a_bad_start_has_no_moon() {
        // New moon 6 January 2000 18:14 UTC, first quarter the 14th, full the 21st, last quarter the 28th.
        let phase = |start: &str| position(start, 0.0, 0.0, 0.0, 0.0).unwrap().phase;
        assert!((phase("2000-01-14T13:34") - 0.25).abs() < 0.01);
        assert!((phase("2000-01-21T04:40") - 0.5).abs() < 0.01);
        assert!((phase("2000-01-28T07:57") - 0.75).abs() < 0.01);
        let new = phase("2000-01-06T18:14");
        assert!(new < 0.01 || new > 0.99, "{new}");
        assert_eq!(position("soon", 0.0, 0.0, 0.0, 0.0), None);
    }
}
