//! The foam field on the GPU (`naval_ocean::foam_sim`): its foam stays on its water when a theatre re-centres the scene
//! or re-lays it on another chart, and under a time scale it lives as long in game seconds as at game speed.
use native_offscreen::Gpu;
use naval_ocean::{
    foam_sim::FoamGrid,
    ship_wake::{ShipWake, WakeCamera, WakeInputs, WakeSplash},
};

/// A small field: 256² cells of 1.5 m.
const GRID: FoamGrid = FoamGrid {
    cells: 256,
    cell: 1.5,
};
const SPLASH: [f64; 2] = [20.3, -11.6];

/// Both states of the field, layer after layer.
const READ: &str = r#"
@group(0) @binding(0) var states: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> texels: array<vec4<f32>>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = textureDimensions(states).x;
    if id.x >= n || id.y >= n { return; }
    texels[(id.z * n + id.y) * n + id.x] = textureLoad(states, vec2<i32>(id.xy), i32(id.z), 0);
}
"#;

/// The field's foam: the middle of its thickness (scene m) and its totals of thickness and residue (summed over cells).
#[derive(Debug)]
struct Foam {
    middle: [f64; 2],
    thickness: f64,
    residue: f64,
}

/// The newest state read back and placed on the scene as the surface places it (`FoamSim::rows`: the window's centre
/// cell's corner and texel, a cell's edge and the cells along an edge).
fn foam(gpu: &Gpu, wake: &ShipWake) -> Foam {
    let d = &gpu.device;
    let states = wake.foam_sim.states().expect("the field has stepped");
    let rows = wake.foam_sim.rows(0);
    let n = rows[2][1] as usize;
    let size = (2 * n * n * 16) as u64;
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(READ.into()),
    });
    let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let texels = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(states),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: texels.as_entire_binding(),
            },
        ],
    });
    let mut encoder = d.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        let groups = (n as u32).div_ceil(8);
        pass.dispatch_workgroups(groups, groups, 2);
    }
    encoder.copy_buffer_to_buffer(&texels, 0, &read, 0, size);
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let all: Vec<[f32; 4]> = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
    let layer = &all[rows[0][1] as usize * n * n..][..n * n];
    let (corner, centre, cell) = (rows[1], [rows[1][2], rows[1][3]], f64::from(rows[2][0]));
    // A texel's cell is the one in the window it stands for, at most half the window from the centre cell.
    let place = |texel: usize, axis: usize| {
        let off = (texel as i64 - centre[axis] as i64).rem_euclid(n as i64);
        let off = if off >= n as i64 / 2 {
            off - n as i64
        } else {
            off
        };
        f64::from(corner[axis]) + (off as f64 + 0.5) * cell
    };
    let (mut sum, mut thickness, mut residue) = ([0.0; 2], 0.0, 0.0);
    for j in 0..n {
        for i in 0..n {
            let texel = layer[j * n + i];
            let t = f64::from(texel[0]);
            sum[0] += t * place(i, 0);
            sum[1] += t * place(j, 1);
            thickness += t;
            residue += f64::from(texel[2]);
        }
    }
    Foam {
        middle: [sum[0] / thickness, sum[1] / thickness],
        thickness,
        residue,
    }
}

/// A wake with the field and no hulls, its window about the water under a camera looking straight down at `at`.
fn wake(gpu: &Gpu) -> ShipWake {
    let mut wake = ShipWake::new(&gpu.device, 0, 0.1);
    wake.set_foam_grid(Some(GRID));
    wake
}

/// One frame of `dt` game seconds, the camera over `at` (scene m), `splash` falling.
fn frame(gpu: &Gpu, wake: &mut ShipWake, dt: f64, at: [f64; 2], splash: Option<[f64; 2]>) {
    let splashes: Vec<WakeSplash> = splash
        .map(|[x, z]| WakeSplash {
            sequence: 1,
            x,
            z,
            caliber_m: 0.38,
        })
        .into_iter()
        .collect();
    wake.update(&WakeInputs {
        ships: &[],
        dt,
        splashes: &splashes,
        torpedoes: &[],
        sea_height: 0.0,
        camera: WakeCamera {
            eye: [at[0], 120.0, at[1]],
            forward: [0.0, -1.0, 0.0],
            fov_y: 52_f64.to_radians(),
            zoom: 1.0,
            drawing_height: 800.0,
        },
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    wake.encode(&gpu.device, &gpu.queue, &mut encoder);
    gpu.queue.submit([encoder.finish()]);
}

/// A re-lay that moves the scene's metres by a shift, turning nothing.
struct Shift([f64; 2]);

impl naval_render::relay::SceneRelay for Shift {
    fn point(&self, p: [f64; 3]) -> [f64; 3] {
        [p[0] + self.0[0], p[1], p[2] + self.0[1]]
    }
    fn vector(&self, _: [f64; 3], v: [f64; 3]) -> [f64; 3] {
        v
    }
    fn heading(&self, _: [f64; 3], heading: f64) -> f64 {
        heading
    }
}

fn near(a: [f64; 2], b: [f64; 2], within: f64) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= within
}

#[test]
fn the_field_keeps_its_foam_on_its_water_across_a_re_centre_and_a_re_lay() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut at = [5.4, -3.9];
    let mut first = wake(&gpu);
    frame(&gpu, &mut first, 0.0, at, Some(SPLASH));
    for _ in 0..180 {
        frame(&gpu, &mut first, 1.0 / 30.0, at, None);
    }
    let laid = foam(&gpu, &first);
    assert!(laid.thickness > 100.0, "the splash laid foam: {laid:?}");
    assert!(
        near(laid.middle, SPLASH, 6.0),
        "the field holds it where it fell: {laid:?}"
    );
    // A re-centre by a shift of no whole number of cells (37.6 and −21.3 of them).
    let shift = [37.6 * GRID.cell, -21.3 * GRID.cell];
    let mut second = wake(&gpu);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    second.take_fields_from(&gpu.device, &mut encoder, &mut first, shift);
    gpu.queue.submit([encoder.finish()]);
    let moved = |p: [f64; 2], by: [f64; 2]| [p[0] + by[0], p[1] + by[1]];
    let carried = foam(&gpu, &second);
    assert!(
        near(carried.middle, moved(laid.middle, shift), 1e-3)
            && (carried.thickness - laid.thickness).abs() < 1e-6 * laid.thickness,
        "the re-centre carries the field as it was: {carried:?} from {laid:?}"
    );
    // Stepped on in the new scene, its window keeps its cells: the foam stays on its water, not cleared.
    at = moved(at, shift);
    for _ in 0..3 {
        frame(&gpu, &mut second, 1.0 / 30.0, at, None);
    }
    let stepped = foam(&gpu, &second);
    assert!(
        near(stepped.middle, carried.middle, 0.5 * GRID.cell)
            && stepped.thickness > 0.95 * carried.thickness,
        "after the re-centre: {stepped:?} from {carried:?}"
    );
    // A re-lay onto another chart, by a shift of no whole number of cells, then stepped on there.
    let relay = [-12.45 * GRID.cell, 8.7 * GRID.cell];
    let mut third = wake(&gpu);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    third.relay_fields_from(&gpu.device, &mut encoder, &mut second, &Shift(relay));
    gpu.queue.submit([encoder.finish()]);
    at = moved(at, relay);
    for _ in 0..3 {
        frame(&gpu, &mut third, 1.0 / 30.0, at, None);
    }
    let relaid = foam(&gpu, &third);
    assert!(
        near(relaid.middle, moved(stepped.middle, relay), 0.5 * GRID.cell)
            && relaid.thickness > 0.95 * stepped.thickness,
        "after the re-lay: {relaid:?} from {stepped:?}"
    );
}

#[test]
fn a_time_scale_keeps_the_foams_life_in_game_seconds() {
    let gpu = Gpu::headless().expect("a GPU");
    let at = [5.4, -3.9];
    // 30 game seconds after the splash fell, at 30 fps.
    let run = |scale: f64| {
        let mut wake = wake(&gpu);
        frame(&gpu, &mut wake, 0.0, at, Some(SPLASH));
        let dt = scale / 30.0;
        for _ in 0..(30.0 / dt).round() as usize {
            frame(&gpu, &mut wake, dt, at, None);
        }
        foam(&gpu, &wake)
    };
    let game = run(1.0);
    // Up to 3× three fixed steps a frame keep up: the field is game speed's, bit for bit.
    let three = run(3.0);
    assert_eq!(
        (three.thickness, three.residue),
        (game.thickness, game.residue)
    );
    // Faster, longer steps share the frames' time. Dropping it instead (three fixed steps a frame) left 1.7 times game
    // speed's foam at 4×, 3.7 at 15× and 4.7 at 120×, and from 0.87 of its residue down to 0.06. Longer steps keep a
    // little more than game speed's (native/docs/ocean/foam.md gives the shares). Not from the cells' rounding: a 30 Hz step
    // changes thick foam by about one fp16 unit, but each cell is rounded up or down at random (`dithered` in
    // foam_sim.wgsl), so it neither drains nor holds the foam however many steps run. What is left is the longer steps'
    // own error: each takes the foam's life and the splash's landing once for all of its time.
    for scale in [4.0, 15.0, 60.0, 120.0] {
        let fast = run(scale);
        let (thickness, residue) = (fast.thickness / game.thickness, fast.residue / game.residue);
        assert!(
            (0.8..1.6).contains(&thickness)
                && (0.8..1.6).contains(&residue)
                && near(fast.middle, game.middle, 2.0),
            "at {scale}x the foam lives as at game speed: {thickness:.2} of its thickness, {residue:.2} of its \
             residue ({fast:?} against {game:?})"
        );
    }
}
