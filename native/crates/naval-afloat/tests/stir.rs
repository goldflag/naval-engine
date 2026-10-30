//! What a hull stirs (`naval_afloat::stir`): a hull whose origin is not amidships has her bow waves' stem and her bow
//! spray's where the sea about her and the wake's generators put her bow, all from her origin's pose.

use glam::DVec3;
use naval_afloat::{Afloat, Calm, Course, Hull};
use naval_ocean::bow_waves::waterline_stem;
use naval_ocean::hull_profile::PROFILE_LEVELS;
use naval_ocean::hull_sea::{HULL_SEA_SLOTS, hull_footprints};
use naval_water_effects::bow_spray::bow_shape;

/// Under way across the sea, her heading 0.6 rad.
const COURSE: Course = Course {
    x: 340.0,
    z: -215.0,
    heading: 0.6,
    speed: 10.0,
};

/// A 114 m destroyer's waterline, her waterplane's centre at `centre` in ship space, afloat on `COURSE` in a calm.
fn afloat(centre: [f64; 2]) -> Afloat {
    let mut afloat = Afloat::new(
        "offset",
        Hull {
            centre,
            ..Hull::new(114.0, 12.0, 4.0)
        },
    )
    .unwrap();
    afloat.update(COURSE, &Calm, 0.0);
    afloat
}

#[test]
fn an_offset_centre_keeps_her_bow_at_her_bow() {
    let ((fx, fz), _) = COURSE.axes();
    let forward = DVec3::new(fx, 0.0, fz);
    // Her stem ahead of her waterplane's centre (m), with her origin amidships: about half her length.
    let ahead = -waterline_stem(afloat([0.0, 0.0]).stir().profile());
    assert!((ahead - 57.0).abs() < 3.0, "{ahead} m");
    for centre in [[0.0, 0.0], [0.0, 50.0], [0.0, -30.0]] {
        let afloat = afloat(centre);
        let stir = afloat.stir();
        assert_eq!(stir.wake_ship().center_z, centre[1]);
        // Where the sea about her puts her bow: her footprint's centre, her stem ahead of it.
        let footprint =
            hull_footprints(&[stir.sea_hull()], (COURSE.x, COURSE.z), HULL_SEA_SLOTS)[0];
        let bow = DVec3::new(footprint.x, 0.0, footprint.z) + forward * ahead;
        // Where the bow waves and the bow spray put it: her profile's stem at the waterline, from her origin's pose.
        let pose = afloat.pose();
        let profile = stir.profile();
        let spray = bow_shape(profile).stems[(PROFILE_LEVELS - 1) / 2];
        for (what, z) in [
            ("bow waves'", waterline_stem(profile)),
            ("bow spray's", spray),
        ] {
            let stem = pose.position() + pose.rotate(DVec3::new(0.0, 0.0, z));
            let off = (stem.x - bow.x).hypot(stem.z - bow.z);
            assert!(
                off < 0.01,
                "centre {centre:?}: the {what} stem {off} m off her bow"
            );
        }
    }
}
