//! The world under the scenario editor's globe on the GPU (`shaders/globe_world.wgsl`; native/docs/ui-contract/maps.md,
//! "The editor's globe"): the disc and space round it, the world chart's depth bands, graticule, land, coasts and reefs
//! ([`crate::world`]'s meshes, uploaded once and projected per vertex). In the game the region's own ground is drawn
//! over it, within the reach (naval-client's `ground::pass`), and the scene's marks over that: nothing veils the world
//! outside the reach.
//!
//! A camera change writes one uniform and picks the tiles in view: nothing is tessellated
//! ([`GlobePass::prepare`] times that work, the test `the_world_costs_little_per_camera` prints it).

use std::sync::Arc;

use anyhow::{Result, bail};
use naval_render::{Pipeline, PipelineId, RenderContext, RenderOutput, ShaderError, ShaderFile};
use wgpu::util::DeviceExt;

use crate::globe::{Globe, V3, dot};
use crate::world::{FillMesh, LineMesh, Lod, TILE_COLUMNS, TILE_ROWS, WorldMeshes, tile_cap};

pub const SHADER_FILE: ShaderFile = naval_render::shader_file!("shaders/globe_world.wgsl");

/// The camera's block (`Camera` in the shader): seven vec4s, in a slot of its own.
const CAMERA_SIZE: u64 = 7 * 16;
/// Each layer's block (`Layer`): two vec4s, at the uniform offset alignment.
const LAYER_SIZE: u64 = 2 * 16;
const SLOT: u64 = 256;
/// Layer slots: the four depth bands, the land, the graticule, the coast, the reefs.
const LAYERS: u64 = 8;
const UNIFORM_SIZE: u64 = SLOT * (1 + LAYERS);
/// Below this many metres to a CSS pixel the near layers (Natural Earth's 1:10m) are drawn; above, the far.
pub const NEAR_BELOW_M_PER_PX: f64 = 6000.0;
/// The reefs are drawn below this many metres to a CSS pixel (the chart's 1.2 px to the nautical mile).
pub const REEFS_BELOW_M_PER_PX: f64 = 1500.0;
/// The graticule is every 30° where its 10° squares would be smaller than this, CSS pixels.
const GRATICULE_10_PX: f64 = 48.0;

/// The bind group layout the globe's pipelines are built against: the camera and a layer (by dynamic offset).
pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let uniform = |binding, dynamic| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globe world"),
        entries: &[uniform(0, false), uniform(1, true)],
    })
}

/// The globe's three pipelines for one output format.
#[derive(Clone, Copy, Debug)]
pub struct GlobePipelines {
    pub base: PipelineId,
    pub fill: PipelineId,
    pub line: PipelineId,
}

#[derive(Clone, Copy)]
enum Kind {
    Base,
    Fill,
    Line,
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    kind: Kind,
    cache: Option<&wgpu::PipelineCache>,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("globe world"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let fill_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    })];
    let line_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: 24,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
    })];
    let (label, vertex, fragment, buffers): (_, _, _, &[Option<wgpu::VertexBufferLayout<'_>>]) =
        match kind {
            Kind::Base => ("globe base", "vs_full", "fs_base", &[]),
            Kind::Fill => ("globe fill", "vs_fill", "fs_fill", &fill_buffers),
            Kind::Line => ("globe line", "vs_line", "fs_line", &line_buffers),
        };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex),
            buffers,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache,
    })
}

/// Registers the globe's pipelines for each of `formats` on a renderer's registry.
pub fn register_pipelines(
    ctx: &mut RenderContext<'_>,
    layout: &wgpu::BindGroupLayout,
    formats: &[wgpu::TextureFormat],
) -> Result<Vec<(wgpu::TextureFormat, GlobePipelines)>, ShaderError> {
    ctx.add_shader(SHADER_FILE)?;
    let mut out: Vec<(wgpu::TextureFormat, GlobePipelines)> = Vec::new();
    for &format in formats {
        if out.iter().any(|(f, _)| *f == format) {
            continue;
        }
        let mut add = |name: &str, kind: Kind| {
            let layout = layout.clone();
            ctx.add_pipeline(
                name,
                SHADER_FILE.path,
                Vec::new(),
                Box::new(move |build| {
                    Pipeline::Render(pipeline(
                        build.device,
                        build.module,
                        &layout,
                        format,
                        kind,
                        build.cache,
                    ))
                }),
            )
        };
        let pipelines = GlobePipelines {
            base: add("globe base", Kind::Base)?,
            fill: add("globe fill", Kind::Fill)?,
            line: add("globe line", Kind::Line)?,
        };
        out.push((format, pipelines));
    }
    Ok(out)
}

/// The three pipelines, as drawn with.
pub struct Pipes<'a> {
    pub base: &'a wgpu::RenderPipeline,
    pub fill: &'a wgpu::RenderPipeline,
    pub line: &'a wgpu::RenderPipeline,
}

/// The pipelines built straight on a device, without a registry (tests).
pub fn direct(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> [wgpu::RenderPipeline; 3] {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("globe world"),
        source: wgpu::ShaderSource::Wgsl(SHADER_FILE.source.into()),
    });
    [Kind::Base, Kind::Fill, Kind::Line]
        .map(|kind| pipeline(device, &module, layout, format, kind, None))
}

/// A CSS colour as the page sends it: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()` and `rgba()` (commas or
/// spaces, alpha 0–1 or a percentage), `color(srgb r g b / a)` or `transparent`; linear and premultiplied.
pub fn colour(text: &str) -> Option<[f32; 4]> {
    let text = text.trim();
    let linear = |c: f64| -> f64 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let rgba: [f64; 4] = if text.eq_ignore_ascii_case("transparent") {
        [0.0; 4]
    } else if let Some(hex) = text.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()?;
        let pair = |i: usize| f64::from(digits[i] * 16 + digits[i + 1]) / 255.0;
        let one = |i: usize| f64::from(digits[i] * 17) / 255.0;
        match digits.len() {
            3 => [one(0), one(1), one(2), 1.0],
            4 => [one(0), one(1), one(2), one(3)],
            6 => [pair(0), pair(2), pair(4), 1.0],
            8 => [pair(0), pair(2), pair(4), pair(6)],
            _ => return None,
        }
    } else {
        let open = text.find('(')?;
        let name = text[..open].trim().to_ascii_lowercase();
        let inner = text[open + 1..].strip_suffix(')')?;
        let (body, alpha) = match inner.split_once('/') {
            Some((body, alpha)) => (body, Some(alpha)),
            None => (inner, None),
        };
        let mut parts: Vec<&str> = body
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .collect();
        let number = |p: &str, whole: f64| -> Option<f64> {
            match p.strip_suffix('%') {
                Some(n) => n.parse::<f64>().ok().map(|v| v / 100.0),
                None => p.parse::<f64>().ok().map(|v| v / whole),
            }
        };
        let (scale, first) = match name.as_str() {
            "rgb" | "rgba" => (255.0, 0),
            "color" if parts.first() == Some(&"srgb") => (1.0, 1),
            _ => return None,
        };
        parts.drain(..first);
        let alpha = match (alpha, parts.len()) {
            (Some(a), 3) => number(a.trim(), 1.0)?,
            (None, 4) => number(parts[3], 1.0)?,
            (None, 3) => 1.0,
            _ => return None,
        };
        [
            number(parts[0], scale)?,
            number(parts[1], scale)?,
            number(parts[2], scale)?,
            alpha,
        ]
    };
    if !rgba.iter().all(|v| v.is_finite()) {
        return None;
    }
    let a = rgba[3].clamp(0.0, 1.0);
    Some([
        (linear(rgba[0].clamp(0.0, 1.0)) * a) as f32,
        (linear(rgba[1].clamp(0.0, 1.0)) * a) as f32,
        (linear(rgba[2].clamp(0.0, 1.0)) * a) as f32,
        a as f32,
    ])
}

/// The globe's colours as CSS text, as the page sends them (the game's `MapGlobeStyle`, which its client hands on).
#[derive(Clone, Copy, Debug)]
pub struct StyleText<'a> {
    /// Outside the Earth's disc.
    pub space: &'a str,
    pub land: &'a str,
    pub coast: &'a str,
    /// CSS pixels.
    pub coast_width: f64,
    /// The 200, 1,000, 3,000 and 4,000 m depth bands, shallow first.
    pub depth: [&'a str; 4],
    /// Water shallower than 200 m, and the globe's base.
    pub sea: &'a str,
    pub reef: &'a str,
    pub graticule: &'a str,
}

/// [`StyleText`]'s colours parsed (linear, premultiplied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobeStyle {
    pub space: [f32; 4],
    pub land: [f32; 4],
    pub coast: [f32; 4],
    pub coast_width: f64,
    pub depth: [[f32; 4]; 4],
    pub sea: [f32; 4],
    pub reef: [f32; 4],
    pub graticule: [f32; 4],
}

impl GlobeStyle {
    pub fn parse(style: &StyleText<'_>) -> Result<Self> {
        let get = |name: &str, text: &str| -> Result<[f32; 4]> {
            match colour(text) {
                Some(c) => Ok(c),
                None => bail!("the globe's {name} colour \"{text}\" is not a CSS colour"),
            }
        };
        if !(style.coast_width.is_finite() && style.coast_width >= 0.0) {
            bail!("the globe's coast width");
        }
        Ok(Self {
            space: get("space", style.space)?,
            land: get("land", style.land)?,
            coast: get("coast", style.coast)?,
            coast_width: style.coast_width,
            depth: [
                get("depth", style.depth[0])?,
                get("depth", style.depth[1])?,
                get("depth", style.depth[2])?,
                get("depth", style.depth[3])?,
            ],
            sea: get("sea", style.sea)?,
            reef: get("reef", style.reef)?,
            graticule: get("graticule", style.graticule)?,
        })
    }

    /// The chart's own colours (`naval_render::palette`, the page's `--ui-*` tokens), as the page sends them with its
    /// depth bands at full strength: for a host without a page.
    pub fn chart() -> Self {
        use naval_render::palette::{
            BAR, CHART_COAST, CHART_GRATICULE, CHART_LAND, CHART_SEA, CHART_SEA_1000,
            CHART_SEA_3000, CHART_SEA_4000,
        };
        let style = Self::parse(&StyleText {
            space: BAR,
            land: CHART_LAND,
            coast: CHART_COAST,
            // The chart's `.land` stroke.
            coast_width: 0.8,
            // The chart paints `d0` and `d1` alike, the sea down to 1,000 m.
            depth: [CHART_SEA, CHART_SEA_1000, CHART_SEA_3000, CHART_SEA_4000],
            sea: CHART_SEA,
            reef: CHART_COAST,
            graticule: CHART_GRATICULE,
        })
        .expect("the palette's colours are CSS colours");
        // Its reefs are the coast's colour at 0.7 (`.reef`).
        Self {
            reef: style.coast.map(|c| c * 0.7),
            ..style
        }
    }
}

/// One globe scene's frame: its camera, where it stands on the output and its colours.
pub struct GlobeDraw<'a> {
    pub globe: &'a Globe,
    /// The scene's rect on the output, device pixels: x, y, width, height.
    pub rect: [f64; 4],
    /// The scene's free parts (the live window cut out), device pixels.
    pub clips: &'a [[u32; 4]],
    pub style: &'a GlobeStyle,
}

impl GlobeDraw<'_> {
    /// Device pixels per scene pixel.
    fn device_per_scene(&self) -> f64 {
        self.rect[2] / self.globe.width
    }
    /// Metres to a CSS pixel at the view's centre, `device_scale` device pixels to one.
    pub fn metres_per_css_px(&self, device_scale: f64) -> f64 {
        device_scale / (self.globe.px_per_m * self.device_per_scene())
    }
}

/// Whether a tile reaches the part of the globe the view shows.
fn tile_seen(caps: &[(V3, f64)], tile: u16, up: V3, reach: f64) -> bool {
    let (centre, radius) = caps[tile as usize];
    dot(centre, up).clamp(-1.0, 1.0).acos() <= radius + reach
}

/// The runs of a mesh's tiles in view, merged where they are contiguous.
fn runs(
    tiles: &[crate::world::TileRange],
    caps: &[(V3, f64)],
    up: V3,
    reach: f64,
    out: &mut Vec<std::ops::Range<u32>>,
) {
    out.clear();
    for range in tiles.iter().filter(|t| tile_seen(caps, t.tile, up, reach)) {
        match out.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => out.push(range.start..range.end),
        }
    }
}

struct GpuFill {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}
struct GpuLine {
    segments: wgpu::Buffer,
}
struct GpuLod {
    depth: [Option<GpuFill>; 4],
    land: Option<GpuFill>,
    coast: Option<GpuLine>,
    reefs: Option<GpuLine>,
}
struct GpuWorld {
    id: u64,
    near: Option<GpuLod>,
    far: Option<GpuLod>,
    graticule: [Option<GpuLine>; 2],
}

/// Floats as the GPU reads them.
fn bytes(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_ne_bytes).collect()
}

fn upload_fill(device: &wgpu::Device, mesh: &FillMesh) -> Option<GpuFill> {
    (!mesh.indices.is_empty()).then(|| GpuFill {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe fill vertices"),
            contents: &bytes(mesh.vertices.iter().flatten().copied()),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe fill indices"),
            contents: &mesh
                .indices
                .iter()
                .flat_map(|i| i.to_ne_bytes())
                .collect::<Vec<u8>>(),
            usage: wgpu::BufferUsages::INDEX,
        }),
    })
}
fn upload_line(device: &wgpu::Device, mesh: &LineMesh) -> Option<GpuLine> {
    (!mesh.segments.is_empty()).then(|| GpuLine {
        segments: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe line segments"),
            contents: &bytes(mesh.segments.iter().flatten().copied()),
            usage: wgpu::BufferUsages::VERTEX,
        }),
    })
}
fn upload_lod(device: &wgpu::Device, lod: &Lod) -> GpuLod {
    GpuLod {
        depth: std::array::from_fn(|i| upload_fill(device, &lod.depth[i])),
        land: upload_fill(device, &lod.land),
        coast: upload_line(device, &lod.coast),
        reefs: upload_line(device, &lod.reefs),
    }
}

/// What a frame draws of the world for one scene: each layer's draws (tiles in view), made on a camera change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub near: bool,
    pub reefs: bool,
    /// 0 for every 10°, 1 for every 30°.
    pub graticule: usize,
    pub depth: [Vec<std::ops::Range<u32>>; 4],
    pub land: Vec<std::ops::Range<u32>>,
    pub coast: Vec<std::ops::Range<u32>>,
    pub reef_runs: Vec<std::ops::Range<u32>>,
    pub graticule_runs: Vec<std::ops::Range<u32>>,
}

/// The globe's GPU state, kept across frames: the uniform, and the world's meshes once uploaded.
pub struct GlobePass {
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
    caps: Vec<(V3, f64)>,
    world: Option<GpuWorld>,
    uploads: naval_render::PassUploads,
    /// The last plan and the camera and world it was made for.
    plan: Option<(PlanKey, Plan)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlanKey {
    centre: [f64; 2],
    px_per_m: f64,
    size: [f64; 2],
    scale: f64,
    world: u64,
}

impl GlobePass {
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globe world"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe world"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform,
                        offset: 0,
                        size: wgpu::BufferSize::new(CAMERA_SIZE),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform,
                        offset: 0,
                        size: wgpu::BufferSize::new(LAYER_SIZE),
                    }),
                },
            ],
        });
        Self {
            uniform,
            group,
            caps: (0..(TILE_COLUMNS * TILE_ROWS) as u16)
                .map(tile_cap)
                .collect(),
            world: None,
            uploads: naval_render::PassUploads::new(device, 16 << 10),
            plan: None,
        }
    }

    /// Lets go of the world's buffers (no globe on screen).
    pub fn release(&mut self) {
        self.world = None;
        self.plan = None;
    }

    /// The draws for `draw`'s camera over `meshes` (`world` numbers them): the level of detail, the tiles in view.
    /// Made again only when the camera, the output's scale or the world changed.
    pub fn prepare(
        &mut self,
        draw: &GlobeDraw<'_>,
        meshes: Option<(&WorldMeshes, u64)>,
        device_scale: f64,
    ) -> &Plan {
        let key = PlanKey {
            centre: draw.globe.centre,
            px_per_m: draw.globe.px_per_m,
            size: [draw.globe.width, draw.globe.height],
            scale: draw.device_per_scene() / device_scale,
            world: meshes.map_or(0, |m| m.1),
        };
        if self.plan.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut plan = Plan::default();
            let metres = draw.metres_per_css_px(device_scale);
            plan.near = metres < NEAR_BELOW_M_PER_PX;
            plan.reefs = metres < REEFS_BELOW_M_PER_PX;
            let ten_degrees = 10f64.to_radians() * crate::globe::EARTH_RADIUS_M / metres;
            plan.graticule = usize::from(ten_degrees < GRATICULE_10_PX);
            if let Some((meshes, _)) = meshes {
                let (up, reach) = (draw.globe.up, draw.globe.reach());
                let lod = if plan.near { &meshes.near } else { &meshes.far };
                for (i, mesh) in lod.depth.iter().enumerate() {
                    runs(&mesh.tiles, &self.caps, up, reach, &mut plan.depth[i]);
                }
                runs(&lod.land.tiles, &self.caps, up, reach, &mut plan.land);
                runs(&lod.coast.tiles, &self.caps, up, reach, &mut plan.coast);
                if plan.reefs {
                    runs(&lod.reefs.tiles, &self.caps, up, reach, &mut plan.reef_runs);
                }
                let grid = &meshes.graticule[plan.graticule];
                runs(&grid.tiles, &self.caps, up, reach, &mut plan.graticule_runs);
            }
            self.plan = Some((key, plan));
        }
        &self.plan.as_ref().unwrap().1
    }

    /// The camera's and the layers' uniform for `draw` on an output `size`.
    fn uniform(
        &self,
        draw: &GlobeDraw<'_>,
        size: [u32; 2],
        encode: bool,
        device_scale: f64,
    ) -> Vec<u8> {
        let g = draw.globe;
        let k = draw.device_per_scene();
        let mut out: Vec<f32> = Vec::with_capacity((UNIFORM_SIZE / 4) as usize);
        let mut push = |v: [f32; 4]| out.extend_from_slice(&v);
        let v4 = |v: V3, w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
        push(v4(g.east, 0.0));
        push(v4(g.north, 0.0));
        push(v4(g.up, 0.0));
        push([
            (draw.rect[0] + draw.rect[2] / 2.0) as f32,
            (draw.rect[1] + draw.rect[3] / 2.0) as f32,
            (g.radius_px() * k) as f32,
            if encode { 1.0 } else { 0.0 },
        ]);
        push([size[0] as f32, size[1] as f32, 0.0, 0.0]);
        push(draw.style.space);
        push(draw.style.sea);
        debug_assert_eq!(out.len() as u64 * 4, CAMERA_SIZE);
        let mut block = bytes(out);
        block.resize(SLOT as usize, 0);
        // The layers: a colour and a line's half width (device pixels) and the share of a thin line's colour kept.
        let line = |width_css: f64| {
            let width = width_css * device_scale;
            [(width / 2.0) as f32, width.min(1.0) as f32, 0.0, 0.0]
        };
        let style = draw.style;
        let layers: [([f32; 4], [f32; 4]); LAYERS as usize] = [
            (style.depth[0], [0.0; 4]),
            (style.depth[1], [0.0; 4]),
            (style.depth[2], [0.0; 4]),
            (style.depth[3], [0.0; 4]),
            (style.land, [0.0; 4]),
            (style.graticule, line(1.0)),
            (style.coast, line(style.coast_width)),
            (style.reef, line(1.0)),
        ];
        for (colour, line) in layers {
            let mut slot = bytes(colour.into_iter().chain(line));
            slot.resize(SLOT as usize, 0);
            block.extend(slot);
        }
        block
    }

    /// Draws the world under a globe scene: the disc and space, then the depth bands, the graticule, the land, the coasts
    /// and the reefs as far as their meshes are made. `device_scale` device pixels to a CSS pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_under(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipes: &Pipes<'_>,
        output: RenderOutput<'_>,
        draw: &GlobeDraw<'_>,
        meshes: Option<(&Arc<WorldMeshes>, u64)>,
        device_scale: f64,
    ) {
        if draw.clips.is_empty() {
            return;
        }
        if let Some((m, id)) = meshes
            && self.world.as_ref().is_none_or(|w| w.id != id)
        {
            self.world = Some(GpuWorld {
                id,
                near: None,
                far: None,
                graticule: [
                    upload_line(device, &m.graticule[0]),
                    upload_line(device, &m.graticule[1]),
                ],
            });
            self.plan = None;
        }
        let plan = self
            .prepare(draw, meshes.map(|(m, id)| (&**m, id)), device_scale)
            .clone();
        if let (Some((m, _)), Some(world)) = (meshes, &mut self.world) {
            let slot = if plan.near {
                &mut world.near
            } else {
                &mut world.far
            };
            if slot.is_none() {
                *slot = Some(upload_lod(device, if plan.near { &m.near } else { &m.far }));
            }
        }
        let bytes = self.uniform(
            draw,
            [output.width, output.height],
            !output.format.is_srgb(),
            device_scale,
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("globe world"),
        });
        self.uploads
            .write_buffer(&mut encoder, &self.uniform, 0, &bytes);
        self.uploads.finish(&encoder);
        {
            let mut pass = begin(&mut encoder, output.view);
            for &[x, y, w, h] in draw.clips {
                pass.set_scissor_rect(x, y, w, h);
                pass.set_pipeline(pipes.base);
                pass.set_bind_group(0, &self.group, &[SLOT as u32]);
                pass.draw(0..3, 0..1);
                let Some(world) = &self.world else {
                    continue;
                };
                let lod = if plan.near { &world.near } else { &world.far };
                let layer = |i: u64| [(SLOT * (1 + i)) as u32];
                if let Some(lod) = lod {
                    pass.set_pipeline(pipes.fill);
                    for (i, fill) in lod.depth.iter().enumerate() {
                        draw_fill(
                            &mut pass,
                            &self.group,
                            layer(i as u64),
                            fill.as_ref(),
                            &plan.depth[i],
                        );
                    }
                }
                pass.set_pipeline(pipes.line);
                draw_line(
                    &mut pass,
                    &self.group,
                    layer(5),
                    world.graticule[plan.graticule].as_ref(),
                    &plan.graticule_runs,
                );
                if let Some(lod) = lod {
                    pass.set_pipeline(pipes.fill);
                    draw_fill(
                        &mut pass,
                        &self.group,
                        layer(4),
                        lod.land.as_ref(),
                        &plan.land,
                    );
                    pass.set_pipeline(pipes.line);
                    draw_line(
                        &mut pass,
                        &self.group,
                        layer(6),
                        lod.coast.as_ref(),
                        &plan.coast,
                    );
                    if plan.reefs {
                        draw_line(
                            &mut pass,
                            &self.group,
                            layer(7),
                            lod.reefs.as_ref(),
                            &plan.reef_runs,
                        );
                    }
                }
            }
        }
        queue.submit([encoder.finish()]);
    }
}

fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("globe world"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    })
}

fn draw_fill(
    pass: &mut wgpu::RenderPass<'_>,
    group: &wgpu::BindGroup,
    offset: [u32; 1],
    fill: Option<&GpuFill>,
    runs: &[std::ops::Range<u32>],
) {
    let Some(fill) = fill else { return };
    if runs.is_empty() {
        return;
    }
    pass.set_bind_group(0, group, &offset);
    pass.set_vertex_buffer(0, fill.vertices.slice(..));
    pass.set_index_buffer(fill.indices.slice(..), wgpu::IndexFormat::Uint32);
    for run in runs {
        pass.draw_indexed(run.clone(), 0, 0..1);
    }
}

fn draw_line(
    pass: &mut wgpu::RenderPass<'_>,
    group: &wgpu::BindGroup,
    offset: [u32; 1],
    line: Option<&GpuLine>,
    runs: &[std::ops::Range<u32>],
) {
    let Some(line) = line else { return };
    if runs.is_empty() {
        return;
    }
    pass.set_bind_group(0, group, &offset);
    pass.set_vertex_buffer(0, line.segments.slice(..));
    for run in runs {
        pass.draw(0..6, run.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::globe::EARTH_RADIUS_M;

    #[test]
    fn the_pages_colours_parse() {
        let c = |t: &str| colour(t).unwrap();
        assert_eq!(c("#ffffff"), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(c("#fff"), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(c("#00000080")[3], 128.0 / 255.0);
        let haze = c("rgba(5, 9, 13, 0.35)");
        assert!((haze[3] - 0.35).abs() < 1e-6);
        assert_eq!(c("rgb(255 255 255 / 50%)")[3], 0.5);
        assert_eq!(c("color(srgb 1 1 1 / 0.25)"), [0.25, 0.25, 0.25, 0.25]);
        assert_eq!(c("transparent"), [0.0; 4]);
        for bad in ["", "red", "#12", "rgb(1,2)", "hsl(1 2 3)", "#zzzzzz"] {
            assert!(colour(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_charts_own_colours_are_the_palettes() {
        let chart = GlobeStyle::chart();
        assert_eq!(
            chart.space[3], 1.0,
            "space is opaque: it covers what was under it"
        );
        assert_eq!(chart.sea, chart.depth[0]);
        let reef = colour("rgba(115, 120, 111, 0.7)").unwrap();
        assert!(
            chart
                .reef
                .iter()
                .zip(reef)
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
        let text = StyleText {
            space: "#000",
            land: "#f00",
            coast: "#0f0",
            coast_width: f64::NAN,
            depth: ["#00f"; 4],
            sea: "#00f",
            reef: "#ff0",
            graticule: "transparent",
        };
        assert!(
            GlobeStyle::parse(&text).is_err(),
            "a coast width that is no number"
        );
        let bad = StyleText {
            land: "olive",
            coast_width: 1.0,
            ..text
        };
        let refused = GlobeStyle::parse(&bad).unwrap_err().to_string();
        assert!(
            refused.contains("land") && refused.contains("olive"),
            "{refused}"
        );
    }

    /// The whole Earth about `[0, 0]` on a 1200 by 800 output, land 30° east of the centre and behind it: the land in
    /// front is drawn and the land behind is not, in the style's colours.
    #[test]
    fn a_globe_draws_the_world_in_front_and_nothing_from_behind() {
        let gpu = native_offscreen::Gpu::headless().expect("a GPU adapter");
        let land = vec![
            (crate::world::tests::square(30.0, 0.0, 4.0), true),
            (crate::world::tests::square(175.0, 0.0, 4.0), true),
        ];
        let bytes =
            crate::world::encode(&[("land", "rings", land.clone()), ("overview", "rings", land)]);
        let mut store = crate::world::WorldStore::from_bytes("test".into(), &bytes).unwrap();
        let meshes = store.settle().expect("the meshes");
        let style = GlobeStyle::parse(&StyleText {
            space: "#000000",
            land: "#ff0000",
            coast: "#00ff00",
            coast_width: 1.0,
            depth: ["#0000ff"; 4],
            sea: "#0000ff",
            reef: "#ffff00",
            graticule: "transparent",
        })
        .unwrap();
        let px_per_m = 0.94 * 800.0 / (2.0 * EARTH_RADIUS_M);
        let globe = Globe::new([0.0, 0.0], px_per_m, 1200.0, 800.0);
        let layout = layout(&gpu.device);
        let p = direct(&gpu.device, &layout, native_offscreen::CAPTURE_FORMAT);
        let capture = native_offscreen::Capture::new(&gpu, 1200, 800);
        let clips = [[0, 0, 1200, 800]];
        let draw = GlobeDraw {
            globe: &globe,
            rect: [0.0, 0.0, 1200.0, 800.0],
            clips: &clips,
            style: &style,
        };
        let mut pass = GlobePass::new(&gpu.device, &layout);
        pass.draw_under(
            &gpu.device,
            &gpu.queue,
            &Pipes {
                base: &p[0],
                fill: &p[1],
                line: &p[2],
            },
            RenderOutput {
                view: &capture.view,
                format: native_offscreen::CAPTURE_FORMAT,
                width: 1200,
                height: 800,
            },
            &draw,
            Some((&meshes, store.id)),
            1.0,
        );
        let pixels = capture.read_rgba(&gpu).unwrap();
        let pixel = |x: f64, y: usize| {
            let i = (y * 1200 + x.round() as usize) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let r = globe.radius_px();
        assert_eq!(
            pixel(600.0 + r * 30f64.to_radians().sin(), 400),
            [255, 0, 0],
            "the land in front"
        );
        assert_eq!(pixel(600.0, 400), [0, 0, 255], "the sea at the centre");
        assert_eq!(
            pixel(600.0 + r * 175f64.to_radians().sin(), 400),
            [0, 0, 255],
            "nothing from behind"
        );
        assert_eq!(pixel(5.0, 5), [0, 0, 0], "space round the globe");
        // The whole Earth is the overview's: no reefs, the 10° graticule (65 px a square here).
        let plan = pass.prepare(&draw, Some((&meshes, store.id)), 1.0);
        assert!(!plan.near && !plan.reefs && plan.graticule == 0);
    }

    #[test]
    fn the_camera_block_is_the_shaders() {
        let source = SHADER_FILE.source;
        let block = &source[source.find("struct Camera").unwrap()..];
        let block = &block[..block.find("};").unwrap()];
        assert_eq!(block.matches("vec4<f32>,").count() as u64 * 16, CAMERA_SIZE);
        let layer = &source[source.find("struct Layer").unwrap()..];
        let layer = &layer[..layer.find("};").unwrap()];
        assert_eq!(layer.matches("vec4<f32>,").count() as u64 * 16, LAYER_SIZE);
    }
}
