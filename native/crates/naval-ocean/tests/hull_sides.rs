//! GPU check of the water's memory against a hull's sides (`naval_ocean::hull_sides`, the heights' pass's `sides_main`):
//! a long sea runs past a hull at rest, rising then falling at her stations. The water's height up her side follows the
//! sea; the height it climbed to holds as it falls, then dries downward at `SIDE_DRYING` until the water meets it again;
//! the slap shows while the water rises and only fades while it falls; a paused frame holds every channel; the ships'
//! copy in the heights' last layer is the history; a new hull in the slot starts dry. Needs a GPU adapter.

use std::sync::mpsc;

use bytemuck::Zeroable;
use native_offscreen::Gpu;
use naval_ocean::hull_heights::{HullHeights, PATCH_ALONG};
use naval_ocean::hull_profile::{PROFILE_STATIONS, SimpleHull, WaterlineProfile};
use naval_ocean::hull_sea::{HullFootprint, HullSeaUniforms, HullSeaWave, everywhere_height};
use naval_ocean::hull_sides::{
    SIDE_CHANNELS, SIDE_DRYING, SIDE_STATIONS, SIDES_LAYER, SideChannel, SideHull,
};
use naval_ocean::{QualityTier, WaveField};
use naval_render::{CameraUniforms, MakePipelines};

use super::hull_heights::{flat_field, read_layer};

/// The history (now, climb, slap, rise) of `slot`'s `side` (0 port, 1 starboard) at `station`.
fn read_state(gpu: &Gpu, heights: &HullHeights, slot: u32, side: u32, station: u32) -> [f32; 4] {
    let offset = u64::from((slot * 2 + side) * SIDE_STATIONS + station) * 16;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hull sides read-back"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(heights.side_state(), offset, &buffer, 0, 16);
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let values: &[f32] = bytemuck::cast_slice(&mapped);
    [values[0], values[1], values[2], values[3]]
}

/// Her starboard half-breadth on her design waterline at `z`, between her profile's stations (as the pass reads it).
fn starboard_breadth(profile: &WaterlineProfile, z: f32) -> f64 {
    let level = 3 * PROFILE_STATIONS;
    let u = ((z - profile.start) / profile.spacing).clamp(0.0, PROFILE_STATIONS as f32 - 1.0);
    let k = (u.floor() as usize).min(PROFILE_STATIONS - 2);
    let at = |i: usize| profile.starboard[level + i].max(0.0);
    f64::from(at(k) + (at(k + 1) - at(k)) * (u - k as f32))
}

#[test]
fn the_sides_remember_how_high_the_sea_climbed() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let device = &gpu.device;
    let field = WaveField::new(device, &QualityTier::High.cascades());
    let layout = field.surface_layout();

    // One long sea drawn everywhere, its crests along her length (she heads north, the sea runs east): 1.5 m high,
    // 200 m long, about eleven seconds from crest to crest.
    let waves = [HullSeaWave {
        amplitude: 1.5,
        wavelength: 200.0,
        direction: 0.0,
        phase: 0.4,
    }];
    // Her footprint as the client couples it (heading north: her bow along −z).
    let footprint = HullFootprint {
        x: 0.0,
        z: 0.0,
        bearing: -std::f64::consts::FRAC_PI_2,
        length: 115.0,
        beam: 12.0,
        contact: 1.0,
    };
    let sea_at = |time: f64| {
        let mut hull_sea = HullSeaUniforms::default();
        hull_sea.set(&waves, time, time, 0.0, &[footprint], (0.0, 0.0));
        hull_sea.set_everywhere(true);
        hull_sea
    };
    use wgpu::util::DeviceExt;
    let hull_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hull sea"),
        contents: bytemuck::bytes_of(&sea_at(0.0)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let group = flat_field(device, layout, &hull_buffer);

    let profile = SimpleHull::warship(115.0, 12.0, 4.0).profile();
    let ship = |id| SideHull {
        id,
        position: [0.0, 0.0, 0.0],
        heading: 0.0,
        pitch: 0.0,
        roll: 0.0,
        profile: &profile,
    };
    let mut heights = HullHeights::new(device, layout, MakePipelines::Now);
    let camera = CameraUniforms::zeroed();
    let frame = |heights: &mut HullHeights, time: f64, step: f64| {
        gpu.queue
            .write_buffer(&hull_buffer, 0, bytemuck::bytes_of(&sea_at(time)));
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(heights.encode(
            device,
            &mut naval_render::UploadSink::Queue(&gpu.queue),
            &mut encoder,
            &camera,
            &group,
            step,
            None,
        ));
        gpu.queue.submit([encoder.finish()]);
    };

    // Amidships on her starboard side: where the pass reads the sea.
    let station = SIDE_STATIONS / 2;
    let span = (profile.stern - profile.bow) / SIDE_STATIONS as f32;
    let z = profile.bow + (station as f32 + 0.5) * span;
    let x = starboard_breadth(&profile, z);
    let sea = |time: f64| everywhere_height(0.0, 0.0, &waves, time, x, f64::from(z));

    heights.place_sides(&[ship("fletcher")]);
    assert_eq!(heights.sides().slot("fletcher"), Some(0));
    let step = 0.1;
    let mut time = 100.0;
    frame(&mut heights, time, step);
    let first = read_state(&gpu, &heights, 0, 1, station);
    assert_eq!(first[1], first[0], "she starts dry above the water");
    assert_eq!((first[2], first[3]), (0.0, 0.0), "and with nothing rushing");

    let mut last = first;
    let (mut highest, mut held, mut dried, mut slapped) = (f32::MIN, 0.0f32, false, false);
    for _ in 0..140 {
        time += step;
        frame(&mut heights, time, step);
        let state = read_state(&gpu, &heights, 0, 1, station);
        let [now, climb, slap, rise] = state;
        assert!(
            (f64::from(now) - sea(time)).abs() < 0.02,
            "now {now} m, the sea {} m",
            sea(time)
        );
        let want = now.max(last[1] - SIDE_DRYING * step as f32);
        assert!((climb - want).abs() < 1e-4, "climb {climb}, want {want}");
        if climb > now + 1e-4 {
            // Above the water, it only dries, at the rate.
            assert!(climb < last[1]);
            dried = true;
        }
        held = held.max(climb - now);
        highest = highest.max(climb);
        if now > last[0] + 0.01 && rise > 0.05 {
            assert!(
                slap >= rise - 1e-4,
                "rising at {rise} m/s with a slap of {slap}"
            );
            slapped = true;
        }
        if now < last[0] && rise < 0.0 {
            assert!(slap <= last[2], "the slap grew as the water fell");
        }
        last = state;
    }
    assert!(slapped, "the water never rose against her");
    assert!(dried, "the climb never stood above the water");
    assert!(
        (highest - 1.5).abs() < 0.05,
        "the climb's highest {highest} m, the crest 1.5 m"
    );
    assert!(held > 0.5, "the climb held at most {held} m over the water");

    // Paused: every channel holds.
    frame(&mut heights, time, 0.0);
    assert_eq!(read_state(&gpu, &heights, 0, 1, station), last);

    // The ships' copy: the history's channels, a row each, in the heights' last layer.
    let layer = read_layer(&gpu, heights.texture(), SIDES_LAYER);
    let column = (station + SIDE_STATIONS) as usize;
    let channel = |c: SideChannel| layer[(c as u32 * PATCH_ALONG) as usize + column];
    assert_eq!(SIDE_CHANNELS, 4);
    assert_eq!(
        [
            channel(SideChannel::Now),
            channel(SideChannel::Climb),
            channel(SideChannel::Slap),
            channel(SideChannel::Rise)
        ],
        last
    );

    // Another hull in her slot starts dry.
    heights.place_sides(&[ship("johnston")]);
    assert_eq!(heights.sides().slot("johnston"), Some(0));
    frame(&mut heights, time, step);
    let fresh = read_state(&gpu, &heights, 0, 1, station);
    assert_eq!(
        fresh[1], fresh[0],
        "a new hull remembers none of the last one's sea"
    );
    assert_eq!((fresh[2], fresh[3]), (0.0, 0.0));
}
