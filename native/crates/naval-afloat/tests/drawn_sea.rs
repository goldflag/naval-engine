//! The sea a hull rides against the sea drawn about her, read back from the GPU: the ocean's heights about a coupled
//! hull (`naval_ocean::hull_heights`, the wet band's patch: the surface's own `height_at` at each texel, choppy
//! displacement inverted, the wakes calm) against the sines ([`SeaWaves::height`]) at the same world points, for a
//! 30 m launch, a destroyer and a battleship in a moderate (9 m/s) and a rough (15 m/s) sea, the High tier's field.
//!
//! Coupled (`Ocean::set_hull_sea` with the sines and her footprint, as a host does), the drawn sea under her is the
//! sines plus the field's waves shorter than the split (about half the swell's length): at points a quarter to half
//! a metre RMS, most of the chop; over her waterplane they largely cancel, the plane fitted to the drawn sea within
//! 0.08–0.26 m of the sines' in heave and 0.4–2.8° in roll (the narrower the hull, the more). Uncoupled (the field
//! alone) the CPU knows nothing of the drawn sea: two to five times further off. What the short waves do to her motion
//! is what `DrawnSea`'s read-back adds over the sines alone, measured on the read-back itself (which returns the
//! drawn heights at her waterplane's points to a millimetre): a battleship 0.1–0.3 m of heave and a fifth of a degree
//! of roll, a destroyer 0.1–0.3 m and up to 1.2°, a launch up to 0.4 m, 2.3° of pitch and 4° of roll. Needs a GPU
//! adapter; prints what it measures (`-- --nocapture`).
//!
//! [`SeaWaves::height`]: naval_effects::sea::SeaWaves::height

use std::sync::mpsc;

use bytemuck::Zeroable;
use native_offscreen::Gpu;
use naval_afloat::{Afloat, Course, DrawnSea, Hull, SeaSurface, SeaUnder, ocean_waves, swell};
use naval_effects::sea::SeaWaves;
use naval_ocean::calibration::{NORTH_ATLANTIC, scene_parameters};
use naval_ocean::field::HullSeaClock;
use naval_ocean::hull_heights::{HullHeights, HullPatch, PATCH_ACROSS, PATCH_ALONG};
use naval_ocean::hull_sea::{HULL_SEA_SLOTS, HullSeaWave, hull_footprints};
use naval_ocean::{QualityTier, WaveField};
use naval_render::{CameraUniforms, MakePipelines, UploadSink};

/// One layer of the heights as metres, row by row (naval-ocean's `tests/hull_heights.rs`).
fn read_layer(gpu: &Gpu, texture: &wgpu::Texture, layer: u32) -> Vec<f32> {
    let row = PATCH_ALONG * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("drawn sea read-back"),
        size: u64::from(padded) * u64::from(PATCH_ACROSS),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(PATCH_ACROSS),
            },
        },
        wgpu::Extent3d {
            width: PATCH_ALONG,
            height: PATCH_ACROSS,
            depth_or_array_layers: 1,
        },
    );
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
    let mut values = Vec::with_capacity((PATCH_ALONG * PATCH_ACROSS) as usize);
    for chunk in mapped.chunks(padded as usize) {
        values.extend(bytemuck::cast_slice::<u8, f32>(&chunk[..row as usize]));
    }
    values
}

/// The drawn height at world (x, z) off a full-resolution patch, bilinearly as the ships read it.
fn drawn_at(patch: &HullPatch, heights: &[f32], x: f64, z: f64) -> f64 {
    let (dx, dz) = (x - patch.x, z - patch.z);
    let along = dx * patch.axis_x + dz * patch.axis_z;
    let across = -dx * patch.axis_z + dz * patch.axis_x;
    let fi = (along / patch.half_length + 1.0) / 2.0 * f64::from(PATCH_ALONG - 1);
    let fj = (across / patch.half_breadth + 1.0) / 2.0 * f64::from(PATCH_ACROSS - 1);
    let (i, j) = (fi.floor() as u32, fj.floor() as u32);
    let (i, j) = (i.min(PATCH_ALONG - 2), j.min(PATCH_ACROSS - 2));
    let (s, t) = (fi - f64::from(i), fj - f64::from(j));
    let at = |i: u32, j: u32| f64::from(heights[(j * PATCH_ALONG + i) as usize]);
    let top = at(i, j) * (1.0 - s) + at(i + 1, j) * s;
    let bottom = at(i, j + 1) * (1.0 - s) + at(i + 1, j + 1) * s;
    top * (1.0 - t) + bottom * t
}

/// The sea drawn about one hull lying at [`AT`]: the High tier's wave field for `wind`, coupled to the sines and her
/// footprint or not, and the heights' patch about her.
struct Drawn {
    parameters: naval_ocean::OceanParameters,
    sea: SeaWaves,
    waves: Vec<HullSeaWave>,
    field: WaveField,
    heights: HullHeights,
    footprints: Vec<naval_ocean::hull_sea::HullFootprint>,
    patch: HullPatch,
    afloat: Afloat,
}

/// Where the measured hulls lie: across the sea (her heading 0.6 rad, the waves running toward 35°), stopped.
const AT: Course = Course {
    x: 340.0,
    z: -215.0,
    heading: 0.6,
    speed: 0.0,
};

impl Drawn {
    fn new(gpu: &Gpu, wind: f64, hull: &Hull, coupled: bool) -> Self {
        let parameters = scene_parameters(&NORTH_ATLANTIC, wind, false);
        let sea = swell(&parameters.waves);
        let waves = if coupled {
            ocean_waves(&sea).expect("a sea").to_vec()
        } else {
            Vec::new()
        };
        let mut afloat = Afloat::new("measured", hull.clone()).unwrap();
        afloat.update(AT, &sea, 0.0);
        let footprints = hull_footprints(&[afloat.stir().sea_hull()], (AT.x, AT.z), HULL_SEA_SLOTS);
        let field = WaveField::new(&gpu.device, &QualityTier::High.cascades());
        let mut heights = HullHeights::new(&gpu.device, field.surface_layout(), MakePipelines::Now);
        heights.place(&footprints);
        let patch = HullPatch::about(&footprints[0]);
        let mut drawn = Self {
            parameters,
            sea,
            waves,
            field,
            heights,
            footprints,
            patch,
            afloat,
        };
        // The first update builds the spectrum; the coupling splits the field's long waves off against it.
        drawn.read(gpu, 0.0);
        drawn
    }

    /// The patch's heights at `time` (s).
    fn read(&mut self, gpu: &Gpu, time: f64) -> Vec<f32> {
        let clock = HullSeaClock {
            swell_time: time,
            racing: 0.0,
        };
        self.field.couple(
            &self.waves,
            time,
            clock,
            &self.footprints,
            (AT.x, AT.z),
            true,
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        self.field
            .update(&gpu.queue, &mut encoder, &self.parameters, time, 1.0 / 60.0);
        assert!(self.heights.encode(
            &gpu.device,
            &mut UploadSink::Queue(&gpu.queue),
            &mut encoder,
            &CameraUniforms::zeroed(),
            self.field.surface_bind_group(),
            0.0,
            None,
        ));
        gpu.queue.submit([encoder.finish()]);
        read_layer(gpu, self.heights.texture(), 0)
    }
}

/// What one comparison found: at points under her waterplane, the drawn sea less the ridden one (RMS and largest) and
/// the ridden one's RMS; over it, the fitted planes' differences (RMS: heave m, pitch and roll radians).
#[derive(Default)]
struct Found {
    points: (f64, f64, f64, f64),
    heave: f64,
    pitch: f64,
    roll: f64,
    /// The ridden plane's own heave, pitch and roll (the largest seen), for scale.
    ridden: (f64, f64, f64),
}

/// The sea about `hull` drawn at `times`, `coupled` (the sines and her footprint) or not, against the sines.
fn measure(gpu: &Gpu, wind: f64, hull: &Hull, coupled: bool, times: &[f64]) -> Found {
    let mut drawn = Drawn::new(gpu, wind, hull, coupled);
    let ((fx, fz), (sx, sz)) = AT.axes();
    let mut found = Found::default();
    let (mut miss, mut ridden, mut n) = (0.0, 0.0, 0.0);
    let (mut heave, mut pitch, mut roll) = (0.0, 0.0, 0.0);
    for &time in times {
        let got = drawn.read(gpu, time);
        let (sea, patch) = (&drawn.sea, &drawn.patch);
        // Every texel under her waterplane's rectangle.
        for j in 0..PATCH_ACROSS {
            for i in 0..PATCH_ALONG {
                let (x, z) = patch.texel(i, j);
                let (dx, dz) = (x - AT.x, z - AT.z);
                let (along, across) = (dx * fx + dz * fz, dx * sx + dz * sz);
                if along.abs() > hull.length / 2.0 || across.abs() > hull.beam / 2.0 {
                    continue;
                }
                let want = sea.height(x, z, time);
                let value = f64::from(got[(j * PATCH_ALONG + i) as usize]);
                miss += (value - want).powi(2);
                ridden += want * want;
                n += 1.0;
                found.points.1 = found.points.1.max((value - want).abs());
            }
        }
        // The planes over her waterplane's own points.
        let waterplane = drawn.afloat.floating().waterplane();
        let points: Vec<_> = waterplane
            .points((AT.x, AT.z), (fx, fz), (sx, sz))
            .collect();
        let cpu: SeaUnder = waterplane.fit(points.iter().map(|&(x, z, _)| sea.height(x, z, time)));
        let gpu_plane: SeaUnder =
            waterplane.fit(points.iter().map(|&(x, z, _)| drawn_at(patch, &got, x, z)));
        let (a, b) = (cpu.level(), gpu_plane.level());
        heave += (b.0 - a.0).powi(2);
        pitch += (b.1 - a.1).powi(2);
        roll += (b.2 - a.2).powi(2);
        found.ridden.0 = found.ridden.0.max(a.0.abs());
        found.ridden.1 = found.ridden.1.max(a.1.abs());
        found.ridden.2 = found.ridden.2.max(a.2.abs());
    }
    let samples = times.len() as f64;
    found.points.0 = (miss / n).sqrt();
    found.points.2 = (ridden / n).sqrt();
    found.points.3 = n / samples;
    found.heave = (heave / samples).sqrt();
    found.pitch = (pitch / samples).sqrt();
    found.roll = (roll / samples).sqrt();
    found
}

/// Her motion riding the drawn sea (the sines and `DrawnSea`'s read-back, copied every fourth frame at 60 a second)
/// against her motion riding the sines alone, over `seconds` after 15 s to settle: the RMS of their difference and of
/// the motion on the sines (heave m, pitch and roll radians). Each read-back is held to the patch it copied.
fn ride(gpu: &Gpu, wind: f64, hull: &Hull, seconds: f64) -> ((f64, f64, f64), (f64, f64, f64)) {
    let mut drawn = Drawn::new(gpu, wind, hull, true);
    let mut reads = DrawnSea::new();
    let mut on_sines = Afloat::new("sines", hull.clone()).unwrap();
    let mut on_drawn = Afloat::new("drawn", hull.clone()).unwrap();
    let (mut miss, mut motion, mut n) = ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 0.0);
    let dt = 1.0 / 60.0;
    for frame in 0..((15.0 + seconds) / dt) as usize {
        let time = 100.0 + frame as f64 * dt;
        if frame % 4 == 0 {
            let got = drawn.read(gpu, time);
            reads.couple(&[("drawn", drawn.footprints[0])], &drawn.sea, time);
            reads.read(&gpu.device, &gpu.queue, &drawn.heights);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            reads.take(&gpu.device);
            assert!(!reads.waiting());
            // What she rides is what is drawn, at her waterplane's points.
            let surface = reads.surface("drawn", &drawn.sea);
            let waterplane = on_drawn.floating().waterplane();
            let ((fx, fz), (sx, sz)) = AT.axes();
            for (x, z, _) in waterplane.points((AT.x, AT.z), (fx, fz), (sx, sz)) {
                let (ridden, shown) = (
                    surface.height(x, z, time),
                    drawn_at(&drawn.patch, &got, x, z),
                );
                assert!(
                    (ridden - shown).abs() < 1e-3,
                    "{ridden} m ridden, {shown} m drawn"
                );
            }
        }
        on_sines.update(AT, &drawn.sea, time);
        on_drawn.update(AT, &reads.surface("drawn", &drawn.sea), time);
        if frame as f64 * dt < 15.0 {
            continue;
        }
        let (a, b) = (on_sines.motion(), on_drawn.motion());
        miss.0 += (a.heave - b.heave).powi(2);
        miss.1 += (a.pitch - b.pitch).powi(2);
        miss.2 += (a.roll - b.roll).powi(2);
        motion.0 += a.heave.powi(2);
        motion.1 += a.pitch.powi(2);
        motion.2 += a.roll.powi(2);
        n += 1.0;
    }
    let rms = |(a, b, c): (f64, f64, f64)| ((a / n).sqrt(), (b / n).sqrt(), (c / n).sqrt());
    (rms(miss), rms(motion))
}

#[test]
fn the_sea_ridden_is_the_sea_drawn() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let times: Vec<f64> = (0..8).map(|k| 100.0 + 7.3 * f64::from(k)).collect();
    let hulls = [
        ("launch", Hull::new(30.0, 6.0, 1.5)),
        ("destroyer", Hull::new(110.0, 11.0, 3.6)),
        ("battleship", Hull::new(250.0, 36.0, 10.0)),
    ];
    for wind in [9.0, 15.0] {
        for (name, hull) in &hulls {
            let coupled = measure(&gpu, wind, hull, true, &times);
            let alone = measure(&gpu, wind, hull, false, &times);
            for (how, f) in [("coupled", &coupled), ("field alone", &alone)] {
                eprintln!(
                    "{wind} m/s, {name}, {how}: points {:.3} m RMS ({:.2} largest) off the sines' {:.3} ({} texels); \
                     plane {:.3} m heave, {:.2}° pitch, {:.2}° roll RMS off the sines' plane (largest {:.2} m, \
                     {:.2}°, {:.2}°)",
                    f.points.0,
                    f.points.1,
                    f.points.2,
                    f.points.3,
                    f.heave,
                    f.pitch.to_degrees(),
                    f.roll.to_degrees(),
                    f.ridden.0,
                    f.ridden.1.to_degrees(),
                    f.ridden.2.to_degrees(),
                );
            }
            // Coupled, the drawn plane under her is the sines' within the field's short waves; alone, it is not.
            assert!(coupled.heave < 0.6 * alone.heave, "{name} at {wind} m/s");
            assert!(
                coupled.heave < 0.3,
                "{name} at {wind} m/s: {} m",
                coupled.heave
            );
            let (miss, motion) = ride(&gpu, wind, hull, 20.0);
            eprintln!(
                "{wind} m/s, {name}, riding the read-back: {:.3} m heave, {:.2}° pitch, {:.2}° roll RMS off her \
                 motion on the sines alone ({:.3} m, {:.2}°, {:.2}° RMS)",
                miss.0,
                miss.1.to_degrees(),
                miss.2.to_degrees(),
                motion.0,
                motion.1.to_degrees(),
                motion.2.to_degrees(),
            );
        }
    }
}

/// The wind changes between a frame and its read-back: what she then rides is the new sea's sines and what the last
/// frame's heights hold beyond the sines they were drawn on (`DrawnSea` keeps the coupling's), with no difference of
/// the two long seas in it to jolt her.
#[test]
fn a_change_of_wind_is_no_jolt() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut drawn = Drawn::new(&gpu, 9.0, &Hull::new(110.0, 11.0, 3.6), true);
    let time = 100.0;
    let got = drawn.read(&gpu, time);
    let mut reads = DrawnSea::new();
    reads.couple(&[("drawn", drawn.footprints[0])], &drawn.sea, time);
    // The wind freshens to 15 m/s before the heights drawn in 9 are copied: the host's sea is the new one.
    let fresh = swell(&scene_parameters(&NORTH_ATLANTIC, 15.0, false).waves);
    reads.read(&gpu.device, &gpu.queue, &drawn.heights);
    reads.couple(&[("drawn", drawn.footprints[0])], &fresh, time + 1.0 / 60.0);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    reads.take(&gpu.device);
    let surface = reads.surface("drawn", &fresh);
    let ((fx, fz), (sx, sz)) = AT.axes();
    let (mut miss, mut seas, mut n) = (0.0_f64, 0.0, 0.0);
    for (x, z, _) in drawn
        .afloat
        .floating()
        .waterplane()
        .points((AT.x, AT.z), (fx, fz), (sx, sz))
    {
        let beyond = drawn_at(&drawn.patch, &got, x, z) - drawn.sea.height(x, z, time);
        miss = miss.max((surface.height(x, z, time) - fresh.height(x, z, time) - beyond).abs());
        seas += (drawn.sea.height(x, z, time) - fresh.height(x, z, time)).powi(2);
        n += 1.0;
    }
    let seas = (seas / n).sqrt();
    eprintln!("a change of wind: {miss:.4} m off, the two seas {seas:.2} m RMS apart");
    // The two seas differ by more than a jolt; none of it is in what she rides.
    assert!(seas > 0.3, "{seas} m");
    assert!(miss < 1e-3, "{miss} m");
}
