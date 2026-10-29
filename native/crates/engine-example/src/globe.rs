//! The round Earth from space (`naval-globe`): the world chart on an orthographic globe, north up, in the chart's own
//! colours. The host reads the chart (the checkout's `public/theatre/world/world-v1.bin`; the crate reads no file) and
//! hands its bytes to a `WorldStore`, which makes the meshes on a thread: until they are in, the globe is a disc of
//! sea. The pipelines are on the renderer's registry, so a WGSL edit hot-reloads as the scene's do.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use naval_globe::globe::V3;
use naval_globe::{
    EARTH_RADIUS_M, Globe, GlobeDraw, GlobePass, GlobePipelines, GlobeStyle, Pipes, WorldStore,
};
use naval_render::{RenderOutput, Renderer};

/// The world chart, in the checkout.
const WORLD_FILE: &str = "public/theatre/world/world-v1.bin";
/// The view's first centre, `[lat, lon]` degrees: the North Atlantic.
const CENTRE: [f64; 2] = [35.0, -35.0];
/// The share of the output's height the whole Earth fills at first, and the zoom's bounds: the Earth at least a fifth
/// of the height, a pixel at least 50 m.
const FILL: f64 = 0.9;
const SMALLEST: f64 = 0.2;
const CLOSEST_M_PER_PX: f64 = 50.0;

pub struct GlobeView {
    pub globe: Globe,
    world: WorldStore,
    pass: GlobePass,
    pipelines: GlobePipelines,
    style: GlobeStyle,
    /// The point under the pointer when its button went down: a drag keeps it there.
    grabbed: Option<V3>,
}

impl GlobeView {
    /// The globe over an output of `format` and `size` (device pixels), its pipelines on `renderer`'s registry.
    pub fn new(
        renderer: &mut Renderer,
        format: wgpu::TextureFormat,
        (width, height): (u32, u32),
    ) -> Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = std::fs::read(root.join(WORLD_FILE)).context(WORLD_FILE)?;
        let world = WorldStore::from_bytes("world-v1".into(), &bytes)?;
        let layout = naval_globe::pass::layout(renderer.device());
        let pipelines =
            naval_globe::pass::register_pipelines(&mut renderer.context(), &layout, &[format])
                .map_err(|e| anyhow!("the globe's shader: {e:?}"))?[0]
                .1;
        let px_per_m = FILL * f64::from(height) / (2.0 * EARTH_RADIUS_M);
        Ok(Self {
            globe: Globe::new(CENTRE, px_per_m, f64::from(width), f64::from(height)),
            world,
            pass: GlobePass::new(renderer.device(), &layout),
            pipelines,
            style: GlobeStyle::chart(),
            grabbed: None,
        })
    }

    /// Waits for the world's meshes (the still).
    pub fn settle(&mut self) -> bool {
        self.world.settle().is_some()
    }

    /// The same view on an output of another size: the Earth keeps its scale.
    pub fn resize(&mut self, (width, height): (u32, u32)) {
        self.globe = Globe::new(
            self.globe.centre,
            self.globe.px_per_m,
            f64::from(width),
            f64::from(height),
        );
    }

    /// The pointer's button down at `at` (device pixels), or up.
    pub fn press(&mut self, at: [f64; 2], down: bool) {
        self.grabbed = down.then(|| self.globe.inverse(at)).flatten();
    }

    /// The pointer moved to `at`: a drag turns the globe under it.
    pub fn drag(&mut self, at: [f64; 2]) {
        if let Some(v) = self.grabbed {
            self.globe = self.globe.grab(v, at);
        }
    }

    /// Zooms by `notches` of the wheel about the point under `at`.
    pub fn zoom(&mut self, notches: f64, at: [f64; 2]) {
        let under = self.globe.inverse(at);
        let smallest = SMALLEST * self.globe.height / (2.0 * EARTH_RADIUS_M);
        let px_per_m =
            (self.globe.px_per_m * 1.25f64.powf(notches)).clamp(smallest, 1.0 / CLOSEST_M_PER_PX);
        self.globe = Globe::new(
            self.globe.centre,
            px_per_m,
            self.globe.width,
            self.globe.height,
        );
        if let Some(v) = under {
            self.globe = self.globe.grab(v, at);
        }
    }

    /// Draws the globe over the whole of `output`: space is opaque, so nothing under it shows. `device_scale` device
    /// pixels to a CSS pixel (the lines' widths and the levels of detail are in CSS pixels).
    pub fn draw(&mut self, renderer: &Renderer, output: RenderOutput<'_>, device_scale: f64) {
        let registry = renderer.pipelines();
        let pipes = Pipes {
            base: registry.render(self.pipelines.base),
            fill: registry.render(self.pipelines.fill),
            line: registry.render(self.pipelines.line),
        };
        let clips = [[0, 0, output.width, output.height]];
        let draw = GlobeDraw {
            globe: &self.globe,
            rect: [0.0, 0.0, f64::from(output.width), f64::from(output.height)],
            clips: &clips,
            style: &self.style,
        };
        let id = self.world.id;
        let meshes = self.world.meshes();
        self.pass.draw_under(
            renderer.device(),
            renderer.queue(),
            &pipes,
            output,
            &draw,
            meshes.as_ref().map(|m| (m, id)),
            device_scale,
        );
    }

    /// The view for the window's title: its centre and the scale there.
    pub fn describe(&self) -> String {
        let [lat, lon] = self.globe.centre;
        format!(
            "the globe about {:.1}°{} {:.1}°{}, {:.1} km a pixel",
            lat.abs(),
            if lat < 0.0 { "S" } else { "N" },
            lon.abs(),
            if lon < 0.0 { "W" } else { "E" },
            0.001 / self.globe.px_per_m
        )
    }
}
