//! The wake field on the GPU, checked as `scripts/diagnostics/ocean-wake.html` checks the web's: a Bismarck-sized hull's
//! bow and stern generators driven as `ShipWake` drives them, the state read back.

use native_offscreen::Gpu;
use naval_ocean::ship_wake::{BREAK_REACH, ShipWake, WakeCamera, WakeInputs, WakeShip};
use naval_ocean::wake::WakeField;
use naval_ocean::wake::generators::GeneratorOptions;

const DT: f64 = 1.0 / 60.0;
const MAX_SPEED: f64 = 15.0;
const LENGTH: f64 = 250.0;
const BEAM: f64 = 36.0;

struct Hull {
    x: f64,
    z: f64,
    heading: f64,
    speed: f64,
    turn: f64,
    bow: u32,
    stern: u32,
}

/// One game frame: move the hull (bow toward −Z at heading 0), drive the generators, step the field.
fn frame(gpu: &Gpu, field: &mut WakeField, hull: &mut Hull, dt: f64) {
    hull.heading += hull.turn * dt;
    hull.x += hull.heading.sin() * hull.speed * dt;
    hull.z -= hull.heading.cos() * hull.speed * dt;
    let ratio = (hull.speed.abs() / MAX_SPEED).min(1.0);
    let active = hull.speed > 0.1;
    field.generators.update(hull.bow, |g| {
        g.active = active;
        g.depth = 0.32 * ratio * ratio;
    });
    field.generators.update(hull.stern, |g| {
        g.active = active;
        g.depth = 0.18 * ratio * ratio;
    });
    for id in [hull.bow, hull.stern] {
        field.generators.set_pose(id, hull.x, hull.z, hull.heading);
    }
    field.foam_strength = 1.2 * ratio;
    field.set_center(hull.x, hull.z);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    field.update(&gpu.queue, &mut encoder, dt);
    gpu.queue.submit([encoder.finish()]);
}

fn run(gpu: &Gpu, field: &mut WakeField, hull: &mut Hull, seconds: f64) {
    for _ in 0..(seconds / DT).round() as usize {
        frame(gpu, field, hull, DT);
    }
}

/// The newest state, RGBA f32 per texel, rows top first.
fn read_state(gpu: &Gpu, field: &WakeField) -> Vec<f32> {
    let n = field.resolution();
    let row = n * 16;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * n),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        field.state_texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(n),
            },
        },
        wgpu::Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    bytemuck::cast_slice::<u8, f32>(&slice.get_mapped_range().unwrap()).to_vec()
}

struct State {
    data: Vec<f32>,
    n: usize,
    origin: [f64; 2],
    cell: f64,
}

impl State {
    fn read(gpu: &Gpu, field: &WakeField) -> Self {
        Self {
            data: read_state(gpu, field),
            n: field.resolution() as usize,
            origin: field.origin(),
            cell: field.world_size() / f64::from(field.resolution()),
        }
    }

    fn at(&self, x: f64, z: f64) -> [f32; 4] {
        let i = ((x - self.origin[0]) / self.cell + self.n as f64 / 2.0).floor();
        let j = ((z - self.origin[1]) / self.cell + self.n as f64 / 2.0).floor();
        if i < 0.0 || j < 0.0 || i >= self.n as f64 || j >= self.n as f64 {
            return [0.0; 4];
        }
        let index = (j as usize * self.n + i as usize) * 4;
        [
            self.data[index],
            self.data[index + 1],
            self.data[index + 2],
            self.data[index + 3],
        ]
    }

    fn max_height(&self) -> f32 {
        self.data.chunks(4).map(|t| t[0].abs()).fold(0.0, f32::max)
    }

    fn max_foam(&self) -> f32 {
        self.data.chunks(4).map(|t| t[2]).fold(0.0, f32::max)
    }

    fn total_foam(&self) -> f64 {
        self.data.chunks(4).map(|t| f64::from(t[2])).sum()
    }

    fn finite(&self) -> bool {
        self.data.iter().all(|v| v.is_finite())
    }
}

/// A point `behind` metres astern of the stern generator.
fn stern_point(hull: &Hull, behind: f64) -> (f64, f64) {
    let back = LENGTH * 0.448 + behind;
    (
        hull.x - hull.heading.sin() * back,
        hull.z + hull.heading.cos() * back,
    )
}

/// Full width at half maximum (m) of the foam across the track at a point.
fn foam_width(state: &State, hull: &Hull, at: (f64, f64)) -> f64 {
    let across = (hull.heading.cos(), hull.heading.sin());
    let profile: Vec<f32> = (-300..=300)
        .map(|s| {
            let s = f64::from(s) * 0.5;
            state.at(at.0 + across.0 * s, at.1 + across.1 * s)[2]
        })
        .collect();
    let peak = profile.iter().copied().fold(0.0, f32::max);
    profile.iter().filter(|&&v| v > peak / 2.0).count() as f64 * 0.5
}

#[test]
fn the_wake_field_behaves_like_the_webs() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut field = WakeField::new(&gpu.device, 512, 1536.0);
    let options = |depth: f64, radius: f64, offset: f64| GeneratorOptions {
        active: false,
        depth,
        radius,
        teleport_threshold: 100.0,
        offset: [0.0, offset],
        breaks_to: [0.0, 0.0],
        break_reach: BEAM * BREAK_REACH,
    };
    let bow = field
        .generators
        .add(options(0.32, BEAM * 0.28, -LENGTH * 0.448))
        .unwrap();
    let stern = field
        .generators
        .add(options(0.18, BEAM * 0.39, LENGTH * 0.448))
        .unwrap();
    let mut hull = Hull {
        x: 0.0,
        z: 0.0,
        heading: 0.0,
        speed: 0.0,
        turn: 0.0,
        bow,
        stern,
    };

    // The web's figures for the same run (`bun scripts/browser/ocean-wake.ts --resolution 512`, 2026-09-30, Chromium on
    // this Apple M5 Pro): its probe reads the sampler on a 512² grid over the field's 1,536 m, the same cells as the
    // state read back here.
    let near = |ours: f64, web: f64, share: f64| (ours - web).abs() <= web.abs() * share;

    run(&gpu, &mut field, &mut hull, 1.0);
    let rest = State::read(&gpu, &field);
    assert_eq!(rest.max_height(), 0.0, "a hull at rest leaves the sea calm");
    assert_eq!(rest.max_foam(), 0.0);

    // Straight at 15 m/s for 30 s.
    hull.speed = MAX_SPEED;
    run(&gpu, &mut field, &mut hull, 30.0);
    let straight = State::read(&gpu, &field);
    let (tx, tz) = stern_point(&hull, 150.0);
    let trail = f64::from(straight.at(tx, tz)[2]);
    let max_height = f64::from(straight.max_height());
    let straight_foam = straight.total_foam();
    eprintln!("straight: trail foam {trail}, max height {max_height}, total foam {straight_foam}");
    assert!(
        near(trail, 0.0802, 0.08),
        "trail foam 150 m astern {trail}, web 0.0802"
    );
    assert!(
        near(max_height, 0.303955, 0.02),
        "max height {max_height}, web 0.3040"
    );
    assert!(
        near(straight_foam, 155.911, 0.03),
        "total foam {straight_foam}, web 155.9"
    );
    assert!(straight.finite() && max_height < 7.99);
    let fresh = foam_width(&straight, &hull, stern_point(&hull, 150.0));
    let older = foam_width(&straight, &hull, stern_point(&hull, 300.0));
    eprintln!("foam widths: {fresh} m then {older} m (web 23.5, 31.5)");
    assert!(older > fresh, "the wake spreads: {fresh} m then {older} m");
    assert!(near(fresh, 23.5, 0.25) && near(older, 31.5, 0.25));
    // The field ahead of the hull is calm; waves reach out abeam of the trail (the Kelvin wedge).
    let ahead = straight.at(
        hull.x + hull.heading.sin() * 400.0,
        hull.z - hull.heading.cos() * 400.0,
    );
    assert!(ahead[0].abs() < 0.01, "calm ahead: {}", ahead[0]);
    let abeam = (0..40)
        .map(|k| straight.at(tx + f64::from(k) * 3.0 + 40.0, tz)[0].abs())
        .fold(0.0, f32::max);
    assert!(abeam > 0.005, "waves abeam of the trail: {abeam}");

    // A paused frame changes nothing.
    for _ in 0..8 {
        frame(&gpu, &mut field, &mut hull, 0.0);
    }
    let paused = State::read(&gpu, &field);
    assert!(paused.data == straight.data, "pause preserves the wake");

    // Turning for 20 s (the web turns its yaw up 0.02 rad/s: the heading down): the old track keeps its foam.
    hull.turn = -0.02;
    run(&gpu, &mut field, &mut hull, 20.0);
    let turned = State::read(&gpu, &field);
    let old_track = f64::from(turned.at(tx, tz)[2]);
    eprintln!("old track foam {old_track} (web 0.00602)");
    assert!(old_track > 0.005 && near(old_track, 0.00602, 0.3));

    // Stopping fades the foam.
    let before_stop = turned.total_foam();
    hull.turn = 0.0;
    hull.speed = 0.0;
    run(&gpu, &mut field, &mut hull, 20.0);
    let stopped = State::read(&gpu, &field).total_foam();
    eprintln!("foam after stop {stopped} (web 17.42)");
    assert!(stopped < before_stop * 0.5 && near(stopped, 17.417, 0.1));

    // Reset clears everything.
    field.reset(&gpu.device);
    let cleared = State::read(&gpu, &field);
    assert!(cleared.data.iter().all(|&v| v == 0.0), "reset clears");

    // A teleport leaves no streak.
    hull.speed = MAX_SPEED;
    run(&gpu, &mut field, &mut hull, 3.0);
    let from = (hull.x, hull.z);
    hull.x += 600.0;
    run(&gpu, &mut field, &mut hull, 3.0);
    let across = State::read(&gpu, &field);
    let middle = across.at(from.0 + 300.0, from.1);
    assert!(
        middle[2] < 0.01 && middle[0].abs() < 0.01,
        "teleport streak: {middle:?}"
    );

    // Breaking: the straight run again with the game's low break threshold, where only breaking can add foam.
    field.reset(&gpu.device);
    hull.heading = 0.0;
    field.foam_break_threshold = 0.015;
    run(&gpu, &mut field, &mut hull, 30.0);
    let breaking = State::read(&gpu, &field).total_foam();
    eprintln!("breaking foam {breaking} (web 578.26)");
    assert!(breaking > 1.2 * straight_foam && near(breaking, 578.26, 0.05));
}

/// What strikes the sea rings it as deep water rings: a crater's waves, read along a line from it, have the local
/// wavelength of the Cauchy–Poisson problem's stationary phase (k = g t² / 4r², the longest waves furthest out), with a
/// hull's breaking threshold set and none of them foaming; and the field steps nothing it was not given.
#[test]
fn a_crater_rings_the_sea_with_deep_water_dispersion() {
    use naval_ocean::wake::impulses::{Strike, WakeImpulse};
    let gpu = Gpu::headless().expect("a GPU");
    let mut field = WakeField::new(&gpu.device, 512, 1536.0);
    field.foam_strength = 1.2;
    field.foam_break_threshold = 0.015;
    let step = |field: &mut WakeField, seconds: f64| {
        for _ in 0..(seconds / DT).round() as usize {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            field.update(&gpu.queue, &mut encoder, DT);
            gpu.queue.submit([encoder.finish()]);
        }
    };
    step(&mut field, 0.5);
    assert_eq!(
        State::read(&gpu, &field).max_height(),
        0.0,
        "a calm field stays calm"
    );
    // Off the field: nothing laid.
    assert!(!field.strike(
        5000.0,
        0.0,
        &Strike::Charge {
            kg: 70.0,
            depth: 0.0
        }
    ));
    // A 6 m crater at the centre, laid at once.
    assert!(field.add_impulse(WakeImpulse {
        displacement: -2.0,
        foam: 1.0,
        ..WakeImpulse::at(0.0, 0.0, 6.0)
    }));
    step(&mut field, 2.0 * DT);
    let laid = State::read(&gpu, &field);
    let bottom = laid.at(0.0, 0.0)[0];
    assert!(
        (-2.2..-1.5).contains(&bottom),
        "the crater's floor {bottom}"
    );
    assert!(laid.at(0.0, 0.0)[2] > 0.7, "white water over it");
    let seconds = 25.0;
    step(&mut field, seconds - 2.0 * DT);
    let state = State::read(&gpu, &field);
    assert!(state.finite());
    // Zero crossings along +x: each half wavelength against 8π r² / (g t²) at its middle.
    let profile: Vec<(f64, f32)> = (0..1600)
        .map(|i| {
            let r = 20.0 + f64::from(i) * 0.25;
            (r, state.at(r, 0.0)[0])
        })
        .collect();
    let crossings: Vec<f64> = profile
        .windows(2)
        .filter(|w| (w[0].1 > 0.0) != (w[1].1 > 0.0))
        .map(|w| w[0].0)
        .collect();
    let mut checked = 0;
    for pair in crossings.windows(2) {
        let middle = 0.5 * (pair[0] + pair[1]);
        let theory = 8.0 * std::f64::consts::PI * middle * middle / (9.81 * seconds * seconds);
        // Only waves the grid carries (6 cells of 3 m and up) and the train has reached.
        if !(24.0..120.0).contains(&theory) {
            continue;
        }
        let measured = 2.0 * (pair[1] - pair[0]);
        eprintln!("at {middle:.0} m: {measured:.1} m, deep water {theory:.1} m");
        assert!(
            (measured / theory - 1.0).abs() < 0.25,
            "at {middle:.0} m: a {measured:.1} m wave where deep water has {theory:.1} m"
        );
        checked += 1;
    }
    assert!(checked >= 3, "{checked} half waves checked: {crossings:?}");
    let ring = profile
        .iter()
        .filter(|(r, _)| (60.0..160.0).contains(r))
        .map(|(_, h)| h.abs())
        .fold(0.0, f32::max);
    eprintln!("ring height {ring} m");
    assert!((0.01..0.5).contains(&ring), "ring height {ring}");
    let foam = (60..160)
        .map(|r| state.at(f64::from(r), 0.0)[2])
        .fold(0.0, f32::max);
    assert!(foam < 0.02, "foam on the rings {foam}");
}

/// A salvo's rings never break while a hull steams by: on Ultra's grid (the finest, whose rings are the steepest) a
/// 38 cm round and a 16-inch HE burster ring the sea for a minute, long after their trains are forgotten, with a
/// battleship at full speed 400 m off setting the hulls' breaking threshold. The white water laid over the craters
/// fades and the rings never add to it, while her track keeps its own foam (how much of it her breaking is,
/// `the_wake_field_behaves_like_the_webs` checks).
#[test]
fn a_salvos_rings_never_break_beside_a_hull() {
    use naval_ocean::wake::impulses::Strike;
    let gpu = Gpu::headless().expect("a GPU");
    let mut field = WakeField::new(&gpu.device, 1024, 1536.0);
    field.foam_break_threshold = 0.015;
    field.set_center(0.0, 0.0);
    let (length, beam, speed) = (250.0, 36.0, 15.0);
    let options = |depth: f64, radius: f64, offset: f64| GeneratorOptions {
        active: true,
        depth,
        radius,
        teleport_threshold: 100.0,
        offset: [0.0, offset],
        breaks_to: [0.0, 0.0],
        break_reach: beam * BREAK_REACH,
    };
    let bow = field
        .generators
        .add(options(0.32, beam * 0.28, -length * 0.448))
        .unwrap();
    let stern = field
        .generators
        .add(options(0.18, beam * 0.39, length * 0.448))
        .unwrap();
    // Across the field 400 m off the falls, heading +X (heading π/2: bow toward +X).
    let mut x = -500.0;
    let z = -400.0;
    let mut frame = |field: &mut WakeField| {
        x += speed * DT;
        for id in [bow, stern] {
            field
                .generators
                .set_pose(id, x, z, std::f64::consts::FRAC_PI_2);
        }
        field.foam_strength = 1.2;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        field.update(&gpu.queue, &mut encoder, DT);
        gpu.queue.submit([encoder.finish()]);
    };
    for _ in 0..(3.0 / DT) as usize {
        frame(&mut field);
    }
    assert!(field.strike(
        0.0,
        0.0,
        &Strike::Round {
            caliber_m: 0.38,
            speed: 400.0
        }
    ));
    assert!(field.strike(
        150.0,
        100.0,
        &Strike::Charge {
            kg: 70.0,
            depth: 0.0
        }
    ));
    let mut previous = f32::INFINITY;
    for second in 1..=60 {
        for _ in 0..(1.0 / DT).round() as usize {
            frame(&mut field);
        }
        if second % 5 != 0 {
            continue;
        }
        let state = State::read(&gpu, &field);
        assert!(state.finite());
        // The rings about both falls, on the side away from her track, out from the middle of each crater.
        let mut foam: f32 = 0.0;
        for (cx, cz) in [(0.0, 0.0), (150.0, 100.0)] {
            for r in (30..300).step_by(2) {
                for a in 0..36 {
                    let angle = f64::from(a) * std::f64::consts::TAU / 36.0;
                    let (px, pz) = (
                        cx + f64::from(r) * angle.cos(),
                        cz + f64::from(r) * angle.sin(),
                    );
                    if pz > -150.0 {
                        foam = foam.max(state.at(px, pz)[2]);
                    }
                }
            }
        }
        eprintln!("{second} s: foam on the rings {foam}");
        // The white water each strike laid over its crater only fades and spreads: the rings never add to it.
        assert!(
            foam <= previous,
            "the rings broke at {second} s: foam {foam}, {previous} before"
        );
        assert!(
            second < 25 || foam < 0.02,
            "foam on the rings at {second} s: {foam}"
        );
        previous = foam;
    }
    // Her track keeps its foam.
    let state = State::read(&gpu, &field);
    let track = (0..200)
        .map(|k| state.at(x - 60.0 - f64::from(k) * 2.0, z)[2])
        .fold(0.0, f32::max);
    eprintln!("foam along her track {track}");
    assert!(track > 0.1, "her track's foam {track}");
}

/// A hull steaming on for [`ShipWake`]: where she is, her heading (bow toward −Z at 0) and speed (m/s).
struct Steamer {
    id: &'static str,
    x: f64,
    z: f64,
    heading: f64,
    speed: f64,
}

/// `seconds` of game frames: each hull steams on, and the wakes take them with `focus` first, as the client does.
fn steam(gpu: &Gpu, wake: &mut ShipWake, hulls: &mut [Steamer], focus: &str, seconds: f64) {
    for _ in 0..(seconds / DT).round() as usize {
        wake_frame(gpu, wake, hulls, focus, |_| {});
    }
}

/// One game frame of [`steam`], `between` called after the wakes' CPU half and before their GPU half (where the client
/// lays what strikes the sea).
fn wake_frame(
    gpu: &Gpu,
    wake: &mut ShipWake,
    hulls: &mut [Steamer],
    focus: &str,
    between: impl FnOnce(&mut ShipWake),
) {
    for hull in hulls.iter_mut() {
        hull.x += hull.heading.sin() * hull.speed * DT;
        hull.z -= hull.heading.cos() * hull.speed * DT;
    }
    let mut ships: Vec<WakeShip<'_>> = hulls
        .iter()
        .map(|h| WakeShip {
            id: h.id,
            position: [h.x, 0.0, h.z],
            heading: h.heading,
            pitch: 0.0,
            roll: 0.0,
            speed: h.speed,
            length: LENGTH,
            beam: BEAM,
            draft: 10.0,
            center_x: 0.0,
            center_z: 0.0,
            forward_speed: MAX_SPEED,
            reverse_speed: 8.0,
            profile: None,
            oil_leak_m3s: 0.0,
        })
        .collect();
    let at = ships.iter().position(|s| s.id == focus).unwrap();
    ships.swap(0, at);
    let eye = [ships[0].position[0] + 300.0, 120.0, ships[0].position[2]];
    wake.update(&WakeInputs {
        ships: &ships,
        dt: DT,
        splashes: &[],
        torpedoes: &[],
        camera: WakeCamera {
            eye,
            forward: [-1.0, 0.0, 0.0],
            fov_y: 50_f64.to_radians(),
            zoom: 1.0,
            drawing_height: 800.0,
        },
        sea_height: 0.0,
    });
    between(wake);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    wake.encode(&gpu.device, &gpu.queue, &mut encoder);
    gpu.queue.submit([encoder.finish()]);
}

/// Mean |height| (m) of the field's newest state over her Kelvin wedge: 100–600 m astern of `hull`, 150 m either side
/// of her track.
fn swell_astern(gpu: &Gpu, field: &WakeField, hull: &Steamer) -> f64 {
    let state = State::read(gpu, field);
    let (ahead, across) = (
        (hull.heading.sin(), -hull.heading.cos()),
        (hull.heading.cos(), hull.heading.sin()),
    );
    let (mut sum, mut count) = (0.0, 0.0);
    for back in (100..=600).step_by(5) {
        for side in (-150..=150).step_by(5) {
            let (back, side) = (f64::from(back), f64::from(side));
            let x = hull.x - ahead.0 * back + across.0 * side;
            let z = hull.z - ahead.1 * back + across.1 * side;
            sum += f64::from(state.at(x, z)[0].abs());
            count += 1.0;
        }
    }
    sum / count
}

/// Ours, and theirs on an opposite course 6.3 km off.
fn two_hulls() -> [Steamer; 2] {
    [
        Steamer {
            id: "ours",
            x: 0.0,
            z: 0.0,
            heading: 0.0,
            speed: MAX_SPEED,
        },
        Steamer {
            id: "theirs",
            x: 6000.0,
            z: -2000.0,
            heading: std::f64::consts::PI,
            speed: MAX_SPEED,
        },
    ]
}

/// Viewing a hull beyond the field and coming back finds the waves the first one raises still about her: the field
/// the view left steps on about her (the standby), and the one made for the distant hull becomes the standby in turn.
/// A theatre re-centre carries both fields into the new scene's metres.
#[test]
fn the_view_going_to_a_distant_hull_and_back_keeps_her_waves() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut wake = ShipWake::new(&gpu.device, 256, 2.0);
    let mut hulls = two_hulls();
    steam(&gpu, &mut wake, &mut hulls, "ours", 30.0);
    let raised = swell_astern(&gpu, wake.field.as_ref().unwrap(), &hulls[0]);
    assert!(raised > 0.004, "her Kelvin wedge: {raised} m");
    assert_eq!(wake.standby_ship(), None, "one field while the view stays");

    steam(&gpu, &mut wake, &mut hulls, "theirs", 15.0);
    assert_eq!(wake.standby_ship(), Some("ours"));
    let centre = wake.field.as_ref().unwrap().center();
    assert!(
        (centre[0] - hulls[1].x).abs() < 1e-9 && (centre[1] - hulls[1].z).abs() < 1e-9,
        "the field drawn follows the distant hull"
    );
    let theirs = swell_astern(&gpu, wake.field.as_ref().unwrap(), &hulls[1]);
    assert!(
        theirs > 0.002,
        "the distant hull's own wedge builds: {theirs} m"
    );

    steam(&gpu, &mut wake, &mut hulls, "ours", DT);
    assert_eq!(
        wake.standby_ship(),
        Some("theirs"),
        "kept about the hull just left"
    );
    let back = swell_astern(&gpu, wake.field.as_ref().unwrap(), &hulls[0]);
    eprintln!("her wedge {raised} m, the distant hull's {theirs} m, hers coming back {back} m");
    assert!(
        back > 0.75 * raised,
        "coming back finds her waves: {back} m against {raised} m before"
    );

    // A re-centre lays the scene's metres again 16.5 km over: the new scene takes both fields, moved with them.
    let shift = [-16_500.0, 3_000.0];
    let mut laid = ShipWake::new(&gpu.device, 256, 2.0);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    laid.take_fields_from(&gpu.device, &mut encoder, &mut wake, shift);
    gpu.queue.submit([encoder.finish()]);
    assert!(wake.field.is_none() && wake.standby_ship().is_none());
    assert_eq!(laid.standby_ship(), Some("theirs"));
    for hull in &mut hulls {
        hull.x += shift[0];
        hull.z += shift[1];
    }
    let moved = swell_astern(&gpu, laid.field.as_ref().unwrap(), &hulls[0]);
    assert!(
        (moved - back).abs() < 1e-6,
        "the field moves with the scene: {moved} m against {back} m"
    );
    steam(&gpu, &mut laid, &mut hulls, "ours", 2.0);
    let on = swell_astern(&gpu, laid.field.as_ref().unwrap(), &hulls[0]);
    assert!(
        on > 0.75 * raised,
        "her waves run on in the new scene: {on} m"
    );
    steam(&gpu, &mut laid, &mut hulls, "theirs", DT);
    let theirs_on = swell_astern(&gpu, laid.field.as_ref().unwrap(), &hulls[1]);
    assert!(
        theirs_on > 0.5 * theirs,
        "and so do the distant hull's: {theirs_on} m against {theirs} m"
    );
}

/// A view that leaves a hull as she goes (sunk, no longer drawn) takes the field she had for the next focus, calm,
/// rather than keeping it for her: the sea is never left without one.
#[test]
fn a_field_left_with_a_hull_that_is_gone_serves_the_next_focus() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut wake = ShipWake::new(&gpu.device, 256, 2.0);
    let mut hulls = two_hulls();
    steam(&gpu, &mut wake, &mut hulls, "ours", 1.0);
    steam(&gpu, &mut wake, &mut hulls[1..], "theirs", DT);
    assert_eq!(wake.standby_ship(), None);
    let field = wake.field.as_ref().expect("a field for the focus");
    let centre = field.center();
    assert!((centre[0] - hulls[1].x).abs() < 1e-9 && (centre[1] - hulls[1].z).abs() < 1e-9);
    assert_eq!(
        State::read(&gpu, field).max_height(),
        0.0,
        "taken calm, not scrolled from where she was"
    );
}

/// A round falling beside a distant hull in the frame the view reaches her rings her field, whether that field is made
/// for her then (the first distant view) or taken calm from the standby (a third hull).
#[test]
fn a_strike_as_the_view_reaches_a_distant_hull_is_kept() {
    use naval_ocean::wake::impulses::Strike;
    let gpu = Gpu::headless().expect("a GPU");
    let mut wake = ShipWake::new(&gpu.device, 256, 2.0);
    let [ours, theirs] = two_hulls();
    let third = Steamer {
        id: "third",
        x: -6000.0,
        z: 3000.0,
        heading: 1.0,
        speed: MAX_SPEED,
    };
    let mut hulls = [ours, theirs, third];
    steam(&gpu, &mut wake, &mut hulls, "ours", 1.0);
    for (focus, at) in [("theirs", 1), ("third", 2)] {
        let (x, z) = (hulls[at].x + 200.0, hulls[at].z);
        wake_frame(&gpu, &mut wake, &mut hulls, focus, |wake| {
            wake.strike(
                x,
                z,
                &Strike::Round {
                    caliber_m: 0.38,
                    speed: 400.0,
                },
            );
        });
        assert!(
            wake.field.as_ref().unwrap().disturbed(),
            "the round beside {focus} rings her field"
        );
    }
}

/// A charge going off in the sea (here a torpedo hit far from any field) leaves its white water, a deposit as wide as
/// its bubble, whether or not a field is there to ring; a round's is its splash's, so its strike lays none.
#[test]
fn a_charge_leaves_white_water_wherever_it_goes_off() {
    use naval_ocean::splash_foam::charge_scale;
    use naval_ocean::wake::impulses::Strike;
    let gpu = Gpu::headless().expect("a GPU");
    let mut wake = ShipWake::new(&gpu.device, 256, 2.0);
    let [ours, theirs] = two_hulls();
    let mut hulls = [ours, theirs];
    steam(&gpu, &mut wake, &mut hulls, "ours", 1.0);
    let round = Strike::Round {
        caliber_m: 0.38,
        speed: 400.0,
    };
    let torpedo = Strike::Charge {
        kg: 350.0,
        depth: 4.0,
    };
    let (x, z) = (hulls[0].x + 9000.0, hulls[0].z);
    wake.strike(x, z, &round);
    assert!(wake.trails.deposits.list.is_empty());
    wake.strike(x, z, &torpedo);
    let deposits = &wake.trails.deposits.list;
    assert_eq!(deposits.len(), 1);
    assert_eq!([deposits[0].x, deposits[0].z], [x, z]);
    assert!((deposits[0].scale - charge_scale(350.0, 4.0)).abs() < 1e-9);
    assert!(
        deposits[0].scale > 1.3,
        "a warhead's white water is wider than a 38 cm round's"
    );
}

/// A re-lay of `shift` metres turning the water by `turn` radians (clockwise), the same everywhere.
struct Relaid {
    shift: [f64; 2],
    turn: f64,
}

impl naval_render::relay::SceneRelay for Relaid {
    fn point(&self, p: [f64; 3]) -> [f64; 3] {
        let [x, z] = self.direction(p, [p[0], p[2]]);
        [x + self.shift[0], p[1], z + self.shift[1]]
    }
    fn vector(&self, _: [f64; 3], v: [f64; 3]) -> [f64; 3] {
        let (sin, cos) = self.turn.sin_cos();
        [v[0] * cos - v[2] * sin, v[1], v[2] * cos + v[0] * sin]
    }
    fn heading(&self, _: [f64; 3], heading: f64) -> f64 {
        heading + self.turn
    }
}

/// A re-lay onto another chart carries both fields by where it lays their centres while the charts turn their water
/// by under half a degree (either way); one that would turn it more leaves them, and the new scene's starts calm.
#[test]
fn a_re_lay_carries_the_fields_it_barely_turns() {
    let gpu = Gpu::headless().expect("a GPU");
    for (turn, carried) in [(0.004, true), (-0.004, true), (0.05, false)] {
        let mut wake = ShipWake::new(&gpu.device, 256, 2.0);
        let mut hulls = two_hulls();
        steam(&gpu, &mut wake, &mut hulls, "ours", 20.0);
        steam(&gpu, &mut wake, &mut hulls, "theirs", 1.0);
        let raised = swell_astern(&gpu, wake.standby_field().unwrap(), &hulls[0]);
        let relay = Relaid {
            shift: [90_000.0, -40_000.0],
            turn,
        };
        let mut laid = ShipWake::new(&gpu.device, 256, 2.0);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        laid.relay_fields_from(&gpu.device, &mut encoder, &mut wake, &relay);
        gpu.queue.submit([encoder.finish()]);
        assert_eq!(laid.standby_ship().is_some(), carried, "turn {turn}");
        if carried {
            // Her place on the new chart, as the relay carries a point.
            use naval_render::relay::SceneRelay;
            let [x, z] = relay.point2([hulls[0].x, hulls[0].z]);
            let moved = Steamer { x, z, ..hulls[0] };
            let kept = swell_astern(&gpu, laid.standby_field().unwrap(), &moved);
            assert!(
                kept > 0.9 * raised,
                "turn {turn}: {kept} m against {raised} m"
            );
        }
    }
}
