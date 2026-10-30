//! A rebuilt scene takes the world's existing fire and funnel smoke, including their ages and local origin.
use glam::{DVec3, Vec3};
use naval_effects::{
    camera::CameraView,
    combat::CombatEffects,
    math::ShipPose,
    ships::{FireState, FireVent, FunnelOutlet, ShipEffectsProfile, ShipFrame},
};
use naval_render::{Camera, FrameTime};
use std::sync::Arc;

#[test]
fn rebuilding_a_scene_keeps_its_smoke_columns_and_rebases_the_released_plume() {
    let profile = Arc::new(ShipEffectsProfile {
        hull_length: 240.0,
        hull_beam: 36.0,
        forward_speed: 15.0,
        reverse_speed: 6.0,
        vents: vec![Some(FireVent {
            position: DVec3::new(0.0, 20.0, 0.0),
            size: [20.0, 10.0, 20.0],
            capacity_m3: 4000.0,
        })],
        funnels: vec![FunnelOutlet {
            id: "funnel".into(),
            position: DVec3::new(0.0, 30.0, 0.0),
            width: 8.0,
            length: 10.0,
            bearing_rad: None,
            smoke: 1.0,
        }],
        ..Default::default()
    });
    let mut ship = ShipFrame::new("ship", profile, ShipPose::default());
    ship.velocity = DVec3::new(10.0, 0.0, 0.0);
    ship.speed = 10.0;
    ship.room_fires = vec![FireState {
        intensity: 0.7,
        ..Default::default()
    }];
    let mut old = CombatEffects::new();
    let camera = Camera::look_at(Vec3::new(400.0, 80.0, 400.0), Vec3::new(400.0, 40.0, 0.0));
    let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
    for tick in 0..400 {
        ship.pose.x = f64::from(tick);
        old.update(
            0.1,
            f64::from(tick) * 0.1,
            &view,
            &[],
            std::slice::from_ref(&ship),
        );
    }
    let count = old.counts();
    assert!(count.fires.smoke > 20);
    let spine = old.funnel_spines().remove(0);
    assert!(spine.samples.iter().any(|p| p.age > 20.0));
    let mut new = CombatEffects::new();
    // From a scene about the world's origin to one about (12 km, −8 km): every point carried 12 km west, 8 km south
    // (the host's `SceneShift::xz` between the two).
    let offset = DVec3::new(-12_000.0, 0.0, 8_000.0);
    new.take_ship_history_from(&mut old, [offset.x, offset.z]);
    assert_eq!(new.counts().fires.smoke, count.fires.smoke);
    let carried = new.funnel_spines().remove(0);
    assert_eq!(carried.samples.len(), spine.samples.len());
    for (from, to) in spine.samples.iter().zip(&carried.samples) {
        assert_eq!(from.age, to.age);
        assert!((from.position + offset).abs_diff_eq(to.position, 1e-8));
    }
    // The first paused frame of the rebuilt scene must not interpret its new origin as a ship teleport.
    ship.pose.x += offset.x;
    ship.pose.z += offset.z;
    new.update(0.0, 40.0, &view, &[], &[ship]);
    assert_eq!(new.counts().fires.smoke, count.fires.smoke);
    assert_eq!(new.funnel_spines()[0].samples.len(), spine.samples.len());
}
