//! Proxy hulls for the stills: each recorded hull drawn as its `naval_ocean::hull_profile::SimpleHull` at its recorded
//! pose, a `naval_render::SceneLayer` in the opaque phase. The ships lane (naval-ships) owns the real models; this only
//! gives the wakes, foam and spray the hull they are fitted to.
//!
//! **Scene-copy seam.** The ocean reads the opaque scene behind the water (straight-through transmission and the thin
//! water column that draws foam along a hull's waterline). Render core II adds copy points for that
//! (`SceneCopies::OPAQUE_COLOR | OPAQUE_DEPTH` and `Phase::Water`, not on master yet). Until it lands, [`HullLayer`] can
//! draw the hulls a second time in its `prepare` into colour and depth targets of its own ([`HullLayer::prepass`]),
//! which the host hands to `Ocean::set_scene`. When render core II merges: drop the prepass, move the ocean to
//! `Phase::Water` with `scene_copies()`, and bind `scene.opaque.color`/`scene.opaque.depth` instead.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use naval_ocean::hull_profile::SimpleHull;
use naval_render::{
    DrawContext, Phase, PipelineId, PrepareContext, RenderContext, SceneLayer, ScenePipeline,
    ShaderError, VertexLayout, shader_file,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct HullVertex {
    position: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct HullLight {
    sun_direction: [f32; 4],
    sun: [f32; 4],
    sky_up: [f32; 4],
    sky_side: [f32; 4],
}

/// Grey hull paint and a scrubbed teak deck (linear albedo).
const SIDE: [f32; 3] = [0.3, 0.31, 0.32];
const BOTTOM: [f32; 3] = [0.18, 0.08, 0.07];
const DECK: [f32; 3] = [0.26, 0.2, 0.13];

/// The sun and sky the proxy hulls are lit by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullLighting {
    pub sun_direction: Vec3,
    /// Sun radiance: colour × intensity.
    pub sun: Vec3,
    pub sky_up: Vec3,
    pub sky_side: Vec3,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

/// One hull to draw this frame: its shape (a mesh is built per distinct shape) and model-to-world matrix.
#[derive(Clone, Copy, Debug)]
pub struct HullDraw {
    pub shape: SimpleHull,
    pub model: Mat4,
}

pub struct HullLayer {
    pipeline: PipelineId,
    light_buffer: wgpu::Buffer,
    light_group: wgpu::BindGroup,
    meshes: Vec<(SimpleHull, Mesh)>,
    instances: wgpu::Buffer,
    draws: Vec<(usize, u32)>,
    prepass: Option<Prepass>,
}

struct Prepass {
    color: wgpu::Texture,
    depth: wgpu::Texture,
    camera: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
}

const MAX_HULLS: u64 = 64;

/// A hull's surface swept from its analytic shape: starboard and port sides from the keel to the deck edge, the deck, the
/// transom. Model frame: x starboard, y up from the design waterline, z aft.
fn hull_mesh(hull: &SimpleHull) -> (Vec<HullVertex>, Vec<u32>) {
    const ALONG: usize = 72;
    const UP: usize = 14;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let stern = hull.length / 2.0;
    // Grid of points on one side: u along the hull from the stem at that height, v from the keel to the deck edge.
    let point = |u: f32, v: f32| -> Vec3 {
        // The deck edge is lowest amidships; heights run keel → edge at the station.
        let y_keel = -hull.draft;
        let z_guess = hull.stem(0.0) + u * (stern - hull.stem(0.0));
        let top = hull.side_height(z_guess);
        let y = y_keel + (top - y_keel) * v;
        let stem = hull.stem(y);
        let z = stem + u * (stern - stem);
        let x = hull
            .half_breadth(z, y.min(hull.side_height(z) - 1e-3))
            .unwrap_or(0.0);
        Vec3::new(x, y, z)
    };
    for side in [1.0_f32, -1.0] {
        let base = vertices.len() as u32;
        let grid: Vec<Vec3> = (0..=ALONG)
            .flat_map(|i| {
                let u = (i as f32 / ALONG as f32).powf(1.3);
                (0..=UP).map(move |j| (u, j as f32 / UP as f32))
            })
            .map(|(u, v)| {
                let p = point(u, v);
                Vec3::new(p.x * side, p.y, p.z)
            })
            .collect();
        for i in 0..=ALONG {
            for j in 0..=UP {
                let at = |a: usize, b: usize| grid[a.min(ALONG) * (UP + 1) + b.min(UP)];
                let du = at(i + 1, j) - at(i.saturating_sub(1), j);
                let dv = at(i, j + 1) - at(i, j.saturating_sub(1));
                let mut normal = du.cross(dv).normalize_or_zero() * side;
                if normal.x * side < 0.0 {
                    normal = -normal;
                }
                let p = grid[i * (UP + 1) + j];
                let albedo = if p.y < -0.3 { BOTTOM } else { SIDE };
                vertices.push(HullVertex {
                    position: p.into(),
                    normal: normal.into(),
                    albedo,
                });
            }
        }
        for i in 0..ALONG {
            for j in 0..UP {
                let a = base + (i * (UP + 1) + j) as u32;
                let b = a + (UP + 1) as u32;
                if side > 0.0 {
                    indices.extend([a, b, a + 1, a + 1, b, b + 1]);
                } else {
                    indices.extend([a, a + 1, b, a + 1, b + 1, b]);
                }
            }
        }
    }
    // The deck: a strip between the two deck edges.
    let base = vertices.len() as u32;
    for i in 0..=ALONG {
        let u = (i as f32 / ALONG as f32).powf(1.3);
        let edge = point(u, 1.0);
        for side in [1.0, -1.0] {
            vertices.push(HullVertex {
                position: [edge.x * side, edge.y, edge.z],
                normal: [0.0, 1.0, 0.0],
                albedo: DECK,
            });
        }
    }
    for i in 0..ALONG as u32 {
        let a = base + i * 2;
        indices.extend([a, a + 1, a + 2, a + 1, a + 3, a + 2]);
    }
    // The transom: a fan at the stern.
    let base = vertices.len() as u32;
    for j in 0..=UP {
        let p = point(1.0, j as f32 / UP as f32);
        for side in [1.0, -1.0] {
            vertices.push(HullVertex {
                position: [p.x * side, p.y, p.z],
                normal: [0.0, 0.0, 1.0],
                albedo: SIDE,
            });
        }
    }
    for j in 0..UP as u32 {
        let a = base + j * 2;
        indices.extend([a, a + 2, a + 1, a + 1, a + 2, a + 3]);
    }
    (vertices, indices)
}

impl HullLayer {
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        let file = shader_file!("shaders/hull.wgsl");
        ctx.add_shader(file)?;
        let light_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("proxy hull light"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let vertex = VertexLayout {
            stride: std::mem::size_of::<HullVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3]
                .to_vec(),
        };
        let instance = VertexLayout {
            stride: 64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4].to_vec(),
        };
        let pipeline = ctx.add_scene_pipeline(
            "proxy hull",
            file.path,
            ScenePipeline {
                vertex_buffers: vec![vertex, instance],
                bind_group_layouts: vec![light_layout.clone()],
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        let light_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("proxy hull light"),
            size: std::mem::size_of::<HullLight>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let light_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("proxy hull light"),
            layout: &light_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: light_buffer.as_entire_binding(),
            }],
        });
        let instances = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("proxy hull instances"),
            size: MAX_HULLS * 64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            light_buffer,
            light_group,
            meshes: Vec::new(),
            instances,
            draws: Vec::new(),
            prepass: None,
        })
    }

    /// Draw the hulls also into colour and depth targets of `width` × `height` in `prepare`, standing in for render core
    /// II's opaque scene copies. Returns their views for `Ocean::set_scene`.
    pub fn prepass(
        &mut self,
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
    ) -> (wgpu::TextureView, wgpu::TextureView) {
        let target = |label: &str, format: wgpu::TextureFormat| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let color = target("hull prepass colour", naval_render::HDR_FORMAT);
        let depth = target("hull prepass depth", naval_render::DEPTH_FORMAT);
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hull prepass camera"),
            size: std::mem::size_of::<naval_render::CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hull prepass camera"),
            layout: camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            }],
        });
        let views = (
            color.create_view(&wgpu::TextureViewDescriptor::default()),
            depth.create_view(&wgpu::TextureViewDescriptor::default()),
        );
        self.prepass = Some(Prepass {
            color,
            depth,
            camera,
            camera_group,
        });
        views
    }

    /// The hulls to draw this frame and the light they are lit by.
    pub fn set_hulls(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hulls: &[HullDraw],
        lighting: HullLighting,
    ) {
        self.draws.clear();
        let mut matrices: Vec<[f32; 16]> = Vec::new();
        for hull in hulls.iter().take(MAX_HULLS as usize) {
            let index = match self
                .meshes
                .iter()
                .position(|(shape, _)| *shape == hull.shape)
            {
                Some(index) => index,
                None => {
                    use wgpu::util::DeviceExt;
                    let (vertices, indices) = hull_mesh(&hull.shape);
                    let mesh = Mesh {
                        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("proxy hull vertices"),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("proxy hull indices"),
                            contents: bytemuck::cast_slice(&indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                        count: indices.len() as u32,
                    };
                    self.meshes.push((hull.shape, mesh));
                    self.meshes.len() - 1
                }
            };
            self.draws.push((index, matrices.len() as u32));
            matrices.push(hull.model.to_cols_array());
        }
        if !matrices.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&matrices));
        }
        let light = HullLight {
            sun_direction: lighting
                .sun_direction
                .normalize_or_zero()
                .extend(0.0)
                .into(),
            sun: lighting.sun.extend(0.0).into(),
            sky_up: lighting.sky_up.extend(0.0).into(),
            sky_side: lighting.sky_side.extend(0.0).into(),
        };
        queue.write_buffer(&self.light_buffer, 0, bytemuck::bytes_of(&light));
    }

    fn record(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, &self.light_group, &[]);
        for &(mesh, instance) in &self.draws {
            let mesh = &self.meshes[mesh].1;
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(
                1,
                self.instances
                    .slice(u64::from(instance) * 64..u64::from(instance + 1) * 64),
            );
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, 0..1);
        }
    }
}

impl SceneLayer for HullLayer {
    fn label(&self) -> &str {
        "proxy hulls"
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let Some(prepass) = &self.prepass else { return };
        ctx.uploads
            .write_buffer(&prepass.camera, 0, bytemuck::bytes_of(ctx.camera));
        let color = prepass
            .color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = prepass
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hull prepass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(naval_render::DEPTH_CLEAR),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &prepass.camera_group, &[]);
        self.record(&mut pass, ctx.pipelines.render(self.pipeline));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            self.record(pass, ctx.pipelines.render(self.pipeline));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_hull_mesh_is_closed_and_the_right_size() {
        let hull = SimpleHull::warship(250.0, 36.0, 9.3);
        let (vertices, indices) = hull_mesh(&hull);
        assert_eq!(indices.len() % 3, 0);
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
        let max_x = vertices.iter().map(|v| v.position[0]).fold(0.0, f32::max);
        let min_z = vertices.iter().map(|v| v.position[2]).fold(0.0, f32::min);
        assert!((max_x - 18.0).abs() < 1.0, "{max_x}");
        assert!(min_z < -125.0, "{min_z}");
    }
}
