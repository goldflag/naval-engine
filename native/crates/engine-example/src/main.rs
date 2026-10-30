//! The engine crates on their own, with no simulation: the sky with its atmosphere and clouds, the sea, a coast cut
//! from a land release and a few combat effects, each a `naval_render::SceneLayer` on one renderer. The template
//! another project copies: see README.md for the crates to depend on and what each layer needs from its host.
//!
//!     bun run cargo -- run -p engine-example --profile dev-client [-- --size 1600x900] [--globe] [--ship <setup.json> …] [--knots <k>] [--hour <h>]
//!     bun run cargo -- run -p engine-example --profile dev-client -- --still <out.png> [--size 1600x900] [--globe] [--ship <setup.json> …] [--knots <k>] [--hour <h>]
//!
//! The window: WASD to fly, Q and E down and up, a mouse button held to look, Shift eight times as fast, the wheel for
//! the speed; T and G an hour on and back, B the next weather, N and M the sea calmer and rougher (the wind 3 m/s
//! down and up), F a gun's blast, splashes, a burst, a shell on the coast and a smoke screen ahead, O the round Earth
//! from space and back (a drag turns it, the wheel zooms about the pointer), Escape to quit. `--globe` starts on the
//! globe. `--exit-after <seconds>` closes it by itself. `--still` renders one frame offscreen from a fixed camera in
//! fixed conditions, the effects 1.2 s after they were spawned (with `--globe`, the whole Earth over the North
//! Atlantic), writes the PNG and exits.
//!
//! `--ship <setup.json>` puts the ship that setup file names (naval-vessel) to sea at `--knots` (15 without it), her
//! hull from her model's numbers (`join.rs`), afloat on the drawn sea with her wake, bow waves, spray and funnel smoke,
//! sailing straight on (`afloat.rs`); each further `--ship` sails abreast of the first. `--afloat <knots>` puts a
//! stand-in ship to sea, sailing a circle, her proxy hull grey. The window and the still then follow the first (the
//! chase camera, off her port bow): `[` and `]` a knot slower and faster, `,` and `.` five, R puts her about, Tab
//! follows the next, C the chase camera back after flying. With `--still`, the frame is the chase camera's after
//! [`AFLOAT_STILL_S`] seconds, the combat effects not spawned; `--wind <m/s>` raises the still's sea (the weather's own
//! without it), `--hour` sets the hour both start at.

mod afloat;
mod coast;
mod fly;
mod globe;
mod join;
mod ship;
mod world;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{FrameInput, FrameTime, RenderOutput, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use fly::Fly;
use globe::GlobeView;
use world::{Conditions, Sail, WEATHERS, World, ahead};

/// Where the window and the still start: 2.5 km off the coast and 700 m up, looking north-north-east down at it.
const EYE: Vec3 = Vec3::new(-1800.0, 700.0, 2500.0);
const TARGET: Vec3 = Vec3::new(-1350.0, 490.0, 1610.0);
/// The still's conditions: mid-afternoon, partly cloudy, the weather's own wind.
const STILL: Conditions = Conditions {
    hour: 15.0,
    weather: 1,
    wind: None,
};
/// The still's frames at 60 a second: the effects spawned at the 18th, so 1.2 s old in the last.
const STILL_FRAMES: u64 = 90;
const STILL_SPAWN: u64 = 18;
/// A still of ships afloat: this long under way at 30 frames a second, so their wakes and smoke stream out astern.
const AFLOAT_STILL_S: f64 = 30.0;
/// The ships' knots without `--knots`.
const KNOTS: f64 = 15.0;
/// The keys that fly the camera: one leaves the chase camera.
const FLY_KEYS: [KeyCode; 6] = [
    KeyCode::KeyW,
    KeyCode::KeyA,
    KeyCode::KeyS,
    KeyCode::KeyD,
    KeyCode::KeyQ,
    KeyCode::KeyE,
];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let option = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let size = match option("--size") {
        Some(s) => {
            let (w, h) = s.split_once('x').context("--size takes <width>x<height>")?;
            (w.parse()?, h.parse()?)
        }
        None => (1600u32, 900u32),
    };
    let globe = args.iter().any(|a| a == "--globe");
    let sail = Sail {
        stand_in: option("--afloat").map(|s| s.parse::<f64>()).transpose()?,
        ships: args
            .windows(2)
            .filter(|w| w[0] == "--ship")
            .map(|w| PathBuf::from(&w[1]))
            .collect(),
        knots: option("--knots")
            .map(|s| s.parse::<f64>())
            .transpose()
            .context("--knots takes a speed in knots")?
            .unwrap_or(KNOTS),
    };
    let hour = option("--hour")
        .map(|s| s.parse::<f64>())
        .transpose()
        .context("--hour takes an hour, 0 to 24")?;
    if let Some(out) = option("--still") {
        let wind = option("--wind").map(|s| s.parse::<f64>()).transpose()?;
        return if globe {
            globe_still(Path::new(&out), size)
        } else {
            let conditions = Conditions {
                hour: hour.unwrap_or(STILL.hour),
                wind,
                ..STILL
            };
            still(Path::new(&out), size, conditions, &sail)
        };
    }
    let exit_after = option("--exit-after")
        .map(|s| s.parse::<f64>())
        .transpose()?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        size,
        globe,
        sail,
        hour: hour.unwrap_or(15.0),
        exit_after,
        running: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

/// One frame offscreen in `conditions`, written to `out`: of the coast and the effects, or with ships afloat (`sail`) of
/// the first from the chase camera.
fn still(
    out: &Path,
    (width, height): (u32, u32),
    conditions: Conditions,
    sail: &Sail,
) -> Result<()> {
    let gpu = Gpu::headless_with(wgpu::Features::TEXTURE_COMPRESSION_BC)?;
    let capture = Capture::new(&gpu, width, height);
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT)
        .map_err(|e| anyhow!("the renderer: {e:?}"))?;
    let mut world = World::new(&mut renderer, conditions, sail)?;
    let mut fly = start(&mut world);
    let afloat = !world.fleet.ships.is_empty();
    let (frames, dt) = match afloat {
        true => ((AFLOAT_STILL_S * 30.0) as u64, 1.0 / 30.0),
        false => (STILL_FRAMES, 1.0 / 60.0),
    };
    let mut squares: Vec<[f64; 3]> = Vec::new();
    for frame in 0..frames {
        if frame == STILL_SPAWN && !afloat {
            world.spawn(ahead(fly.eye, fly.forward()), fly.forward().as_dvec3());
        }
        // A still of ships follows the first.
        chase(&mut world, &mut fly);
        let camera = fly.camera();
        world.advance(&mut renderer, &camera, (width, height), dt);
        // Their motion every 5 s, now and its root mean square over those 5 s: heave up, pitch bow up, roll starboard
        // side up.
        squares.resize(world.fleet.ships.len(), [0.0; 3]);
        for (ship, sum) in world.fleet.ships.iter().zip(&mut squares) {
            let m = ship.afloat.motion();
            for (s, v) in sum
                .iter_mut()
                .zip([m.heave, m.pitch.to_degrees(), m.roll.to_degrees()])
            {
                *s += v * v / 150.0;
            }
            if frame % 150 == 149 {
                println!(
                    "{:5.1} s {}: heave {:+.2} m ({:.2}), pitch {:+.2}° ({:.2}), roll {:+.2}° ({:.2})",
                    (frame + 1) as f64 * dt,
                    ship.name,
                    m.heave,
                    sum[0].sqrt(),
                    m.pitch.to_degrees(),
                    sum[1].sqrt(),
                    m.roll.to_degrees(),
                    sum[2].sqrt()
                );
                *sum = [0.0; 3];
            }
        }
        if frame + 1 == frames {
            renderer.snap_eye_adaptation();
        }
        renderer
            .render(
                FrameInput {
                    camera: &camera,
                    time: FrameTime {
                        seconds: frame as f64 * dt,
                        dt: dt as f32,
                        frame,
                    },
                },
                &mut world.layers(),
                RenderOutput {
                    view: &capture.view,
                    format: CAPTURE_FORMAT,
                    width,
                    height,
                },
            )
            .map_err(|e| anyhow!("frame {frame}: {e:?}"))?;
    }
    save(&gpu, &capture, out)
}

/// The first view: the chase camera's with ships afloat, else 2.5 km off the coast.
fn start(world: &mut World) -> Fly {
    let mut fly = Fly::looking_at(EYE, TARGET);
    chase(world, &mut fly);
    fly
}

/// Put `fly` where the chase camera has the ship followed (`afloat::Sailing::chase`), if one is afloat, keeping its
/// speed; whether one is.
fn chase(world: &mut World, fly: &mut Fly) -> bool {
    let Some(ship) = world.fleet.followed() else {
        return false;
    };
    let (eye, target) = ship.chase();
    *fly = Fly {
        speed: fly.speed,
        ..Fly::looking_at(eye, target)
    };
    true
}

/// The round Earth offscreen, its world chart's meshes made first, written to `out`.
fn globe_still(out: &Path, (width, height): (u32, u32)) -> Result<()> {
    let gpu = Gpu::headless()?;
    let capture = Capture::new(&gpu, width, height);
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT)
        .map_err(|e| anyhow!("the renderer: {e:?}"))?;
    let mut view = GlobeView::new(&mut renderer, CAPTURE_FORMAT, (width, height))?;
    if !view.settle() {
        bail!("the world chart's meshes were not made");
    }
    view.draw(
        &renderer,
        RenderOutput {
            view: &capture.view,
            format: CAPTURE_FORMAT,
            width,
            height,
        },
        1.0,
    );
    save(&gpu, &capture, out)
}

/// Writes the capture to `out` and says how much of it is black.
fn save(gpu: &Gpu, capture: &Capture, out: &Path) -> Result<()> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    capture.save_png(gpu, out)?;
    // A frame gone wrong is black (or a NaN's black) over much of it: say how much, by thirds from the top.
    let pixels = capture.read_rgba(gpu)?;
    let row = capture.width as usize * 4;
    let thirds: Vec<String> = (0..3)
        .map(|k| {
            let band =
                &pixels[k * pixels.len() / 3 / row * row..(k + 1) * pixels.len() / 3 / row * row];
            let dark = band
                .chunks(4)
                .filter(|p| p[0] < 4 && p[1] < 4 && p[2] < 4)
                .count();
            let mean =
                band.chunks(4).map(|p| u64::from(p[1])).sum::<u64>() / (band.len() as u64 / 4);
            format!("mean green {mean}, {dark} black")
        })
        .collect();
    println!("{}: {}", out.display(), thirds.join("; "));
    Ok(())
}

struct App {
    size: (u32, u32),
    /// `--globe`: the window starts on the globe.
    globe: bool,
    /// `--afloat`, `--ship`, `--knots`: what sails.
    sail: Sail,
    /// `--hour`: the hour the window starts at.
    hour: f64,
    exit_after: Option<f64>,
    running: Option<Running>,
    error: Option<anyhow::Error>,
}

/// The window's device, surface, renderer and scene, and what the keys and mouse hold.
struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    world: World,
    fly: Fly,
    /// Whether the view is the chase camera's, following the ship followed (until a fly key or a look leaves it).
    chasing: bool,
    /// The round Earth (O), made the first time it is shown, and whether it is shown.
    globe: Option<GlobeView>,
    on_globe: bool,
    /// The pointer, device pixels.
    cursor: [f64; 2],
    held: HashSet<KeyCode>,
    looking: bool,
    /// Whether winit says the window is hidden: nothing is drawn then.
    occluded: bool,
    started: Instant,
    last: Instant,
    frame: u64,
    /// Redraws the surface gave no texture for (occluded, timed out, reconfigured).
    skipped: u64,
}

impl Running {
    fn new(
        event_loop: &ActiveEventLoop,
        (width, height): (u32, u32),
        on_globe: bool,
        sail: &Sail,
        hour: f64,
    ) -> Result<Self> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("engine example")
                    .with_inner_size(PhysicalSize::new(width, height)),
            )?,
        );
        let instance = wgpu::Instance::new(
            native_offscreen::instance_descriptor()
                .with_display_handle(Box::new(event_loop.owned_display_handle())),
        );
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("no GPU adapter for the window")?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("engine example"),
                // The trees' impostors and the land's photographs in BC7 where the adapter has it.
                required_features: adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC,
                // The effects bind more textures than the default allows.
                required_limits: naval_effects::trial::limits(&adapter),
                ..Default::default()
            }))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("the surface takes no configuration")?;
        // The display transform writes sRGB: an sRGB surface where there is one.
        let caps = surface.get_capabilities(&adapter);
        if let Some(&srgb) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let mut renderer =
            Renderer::new(&device, &queue, config.width, config.height, config.format)
                .map_err(|e| anyhow!("the renderer: {e:?}"))?;
        let mut world = World::new(
            &mut renderer,
            Conditions {
                hour,
                weather: 1,
                wind: None,
            },
            sail,
        )?;
        println!(
            "WASD fly, Q/E down/up, mouse button + drag look, Shift fast, wheel speed; T/G hour, B weather, N/M sea, \
             F effects, O globe (drag turn, wheel zoom), Esc quit"
        );
        if !world.fleet.ships.is_empty() {
            println!(
                "[/] a knot slower/faster, ,/. five, R put about, Tab next ship, C chase camera"
            );
        }
        let now = Instant::now();
        let fly = start(&mut world);
        let chasing = !world.fleet.ships.is_empty();
        let mut running = Self {
            window,
            surface,
            config,
            device,
            queue,
            renderer,
            world,
            fly,
            chasing,
            globe: None,
            on_globe: false,
            cursor: [0.0; 2],
            held: HashSet::new(),
            looking: false,
            occluded: false,
            started: now,
            last: now,
            frame: 0,
            skipped: 0,
        };
        if on_globe {
            running.toggle_globe()?;
        }
        running.title();
        // In front of the terminal that started it: a window behind others (or on another desktop) is occluded and
        // draws nothing.
        running.window.focus_window();
        Ok(running)
    }

    fn title(&self) {
        if let Some(globe) = self.globe.as_ref().filter(|_| self.on_globe) {
            self.window
                .set_title(&format!("engine example: {}", globe.describe()));
            return;
        }
        let c = self.world.conditions;
        let fleet = &self.world.fleet;
        let afloat = fleet.ships.get(fleet.followed).map_or(String::new(), |s| {
            let of = match fleet.ships.len() {
                1 => String::new(),
                n => format!(" ({} of {n})", fleet.followed + 1),
            };
            let chasing = if self.chasing { ", chasing" } else { "" };
            format!(", {} at {:.0} knots{of}{chasing}", s.name, s.knots)
        });
        self.window.set_title(&format!(
            "engine example: {:02.0}:00, {}, wind {:.0} m/s, {:.0} m/s flying{afloat}",
            c.hour.rem_euclid(24.0),
            WEATHERS[c.weather % WEATHERS.len()],
            self.world.scene.wind.0,
            self.fly.speed
        ));
    }

    /// Shows the globe, made the first time, or the scene again.
    fn toggle_globe(&mut self) -> Result<()> {
        if self.globe.is_none() {
            self.globe = Some(GlobeView::new(
                &mut self.renderer,
                self.config.format,
                (self.config.width, self.config.height),
            )?);
        }
        self.on_globe = !self.on_globe;
        self.looking = false;
        // A drag the switch cut short: its button's release goes to the other mode.
        if let Some(globe) = self.globe.as_mut() {
            globe.press(self.cursor, false);
        }
        Ok(())
    }

    fn key(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) -> Result<()> {
        let mut conditions = self.world.conditions;
        match code {
            KeyCode::Escape => event_loop.exit(),
            KeyCode::KeyO => {
                self.toggle_globe()?;
                self.title();
            }
            KeyCode::KeyT => conditions.hour += 1.0,
            KeyCode::KeyG => conditions.hour -= 1.0,
            KeyCode::KeyB => conditions.weather = (conditions.weather + 1) % WEATHERS.len(),
            KeyCode::KeyN => conditions.wind = Some((self.world.scene.wind.0 - 3.0).max(0.0)),
            KeyCode::KeyM => conditions.wind = Some((self.world.scene.wind.0 + 3.0).min(30.0)),
            KeyCode::KeyF => {
                let forward = self.fly.forward();
                self.world
                    .spawn(ahead(self.fly.eye, forward), forward.as_dvec3());
            }
            KeyCode::BracketLeft | KeyCode::BracketRight | KeyCode::Comma | KeyCode::Period => {
                let knots = match code {
                    KeyCode::BracketLeft => -1.0,
                    KeyCode::BracketRight => 1.0,
                    KeyCode::Comma => -5.0,
                    _ => 5.0,
                };
                if let Some(ship) = self.world.fleet.followed() {
                    ship.throttle(knots);
                    self.title();
                }
            }
            KeyCode::KeyR => {
                if let Some(ship) = self.world.fleet.followed() {
                    ship.helm.put_about();
                    self.title();
                }
            }
            KeyCode::Tab => {
                self.world.fleet.follow_next();
                self.chasing = !self.world.fleet.ships.is_empty();
                self.title();
            }
            KeyCode::KeyC => {
                self.chasing = !self.chasing && !self.world.fleet.ships.is_empty();
                self.title();
            }
            _ => {}
        }
        if conditions != self.world.conditions {
            self.world.set_conditions(&self.queue, conditions)?;
            self.title();
        }
        Ok(())
    }

    fn redraw(&mut self) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.skipped += 1;
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => {
                // Nothing waits for a drawable then: without a pause the loop spins on failed acquisitions.
                self.skipped += 1;
                std::thread::sleep(std::time::Duration::from_millis(33));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                bail!("the surface's texture failed validation")
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64().min(0.1);
        self.last = now;
        if let Some(globe) = self.globe.as_mut().filter(|_| self.on_globe) {
            globe.draw(
                &self.renderer,
                RenderOutput {
                    view: &view,
                    format: self.config.format,
                    width: self.config.width,
                    height: self.config.height,
                },
                self.window.scale_factor(),
            );
            self.frame += 1;
            self.window.pre_present_notify();
            self.queue.present(frame);
            return Ok(());
        }
        // A fly key or a look leaves the chase camera where it is.
        if self.chasing && (self.looking || FLY_KEYS.iter().any(|k| self.held.contains(k))) {
            self.chasing = false;
            self.title();
        }
        if !(self.chasing && chase(&mut self.world, &mut self.fly)) {
            self.fly.fly(&self.held, dt as f32);
        }
        let camera = self.fly.camera();
        let (width, height) = (self.config.width, self.config.height);
        self.world
            .advance(&mut self.renderer, &camera, (width, height), dt);
        self.renderer
            .render(
                FrameInput {
                    camera: &camera,
                    time: FrameTime {
                        seconds: now.duration_since(self.started).as_secs_f64(),
                        dt: dt as f32,
                        frame: self.frame,
                    },
                },
                &mut self.world.layers(),
                RenderOutput {
                    view: &view,
                    format: self.config.format,
                    width,
                    height,
                },
            )
            .map_err(|e| anyhow!("frame {}: {e:?}", self.frame))?;
        self.frame += 1;
        if self.frame == 1 {
            // The first frame waits for every layer's pipelines (they compile on the renderer's build threads).
            println!(
                "first frame in {:.1} s",
                self.started.elapsed().as_secs_f64()
            );
        }
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(())
    }
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none() {
            match Running::new(event_loop, self.size, self.globe, &self.sail, self.hour) {
                Ok(running) => self.running = Some(running),
                Err(error) => self.fail(event_loop, error),
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };
        let result = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                Ok(())
            }
            WindowEvent::Resized(size) => {
                running.config.width = size.width.max(1);
                running.config.height = size.height.max(1);
                running.surface.configure(&running.device, &running.config);
                if let Some(globe) = &mut running.globe {
                    globe.resize((running.config.width, running.config.height));
                }
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. } => match event.physical_key {
                PhysicalKey::Code(code) if event.state == ElementState::Pressed => {
                    running.held.insert(code);
                    if event.repeat {
                        Ok(())
                    } else {
                        running.key(code, event_loop)
                    }
                }
                PhysicalKey::Code(code) => {
                    running.held.remove(&code);
                    Ok(())
                }
                PhysicalKey::Unidentified(_) => Ok(()),
            },
            WindowEvent::MouseInput { state, .. } => {
                let down = state == ElementState::Pressed;
                match running.globe.as_mut().filter(|_| running.on_globe) {
                    Some(globe) => globe.press(running.cursor, down),
                    None => running.looking = down,
                }
                Ok(())
            }
            WindowEvent::CursorMoved { position, .. } => {
                running.cursor = [position.x, position.y];
                if let Some(globe) = running.globe.as_mut().filter(|_| running.on_globe) {
                    globe.drag(running.cursor);
                    running.title();
                }
                Ok(())
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                match running.globe.as_mut().filter(|_| running.on_globe) {
                    Some(globe) => globe.zoom(f64::from(notches), running.cursor),
                    None => {
                        running.fly.speed =
                            (running.fly.speed * 1.25f32.powf(notches)).clamp(2.0, 2000.0)
                    }
                }
                running.title();
                Ok(())
            }
            WindowEvent::Occluded(occluded) => {
                running.occluded = occluded;
                println!(
                    "{}",
                    if occluded {
                        "window hidden"
                    } else {
                        "window shown"
                    }
                );
                Ok(())
            }
            WindowEvent::Focused(false) => {
                running.held.clear();
                running.looking = false;
                if let Some(globe) = running.globe.as_mut() {
                    globe.press(running.cursor, false);
                }
                Ok(())
            }
            WindowEvent::RedrawRequested if running.occluded => {
                std::thread::sleep(std::time::Duration::from_millis(33));
                Ok(())
            }
            WindowEvent::RedrawRequested => running.redraw(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let (Some(running), DeviceEvent::MouseMotion { delta }) = (&mut self.running, event)
            && running.looking
        {
            running.fly.look(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            if self
                .exit_after
                .is_some_and(|s| running.started.elapsed().as_secs_f64() > s)
            {
                println!(
                    "{} frames in {:.1} s ({} redraws without a surface texture)",
                    running.frame,
                    running.started.elapsed().as_secs_f64(),
                    running.skipped
                );
                event_loop.exit();
                return;
            }
            running.window.request_redraw();
        }
    }
}
