//! Grass about the camera (native/docs/terrain/trees.md, "Grass"): blades on the ground within [`GRASS_REACH_M`] of the
//! eye, placed each frame on the GPU (`shaders/grass_place.wgsl`: a clump of blades on each cell of a grid fixed to the
//! chart, every cell's near the eye and fewer by the square of the distance beyond [`GRASS_FULL_M`], where the land is
//! grass, in the ground's own colour and as tall as that ground's grass grows) and drawn as geometry through an
//! indirect draw (`shaders/grass.wgsl`: a strip a blade), with the land's groups and lit as the ground is. With the
//! trees (High).

use bytemuck::{Pod, Zeroable};
use naval_render::{
    PipelineId, PrepareContext, RenderContext, ScenePipeline, ShaderDefs, ShaderError,
    VertexLayout, compute_recipe,
};

/// Clumps the placement has room for: about twice the most a view near the ground stands.
pub const GRASS_CAPACITY: u32 = 65_536;
/// A placed clump's bytes (`grass_place.wgsl`'s `Clump`).
pub const CLUMP_BYTES: u64 = 32;
/// Blades a clump, and the vertices each takes (`grass.wgsl`'s `BLADES`, `BLADE_VERTICES`).
pub const BLADES: u32 = 16;
pub const BLADE_VERTICES: u32 = 12;
/// The grid's cell (m).
pub const GRASS_CELL_M: f32 = 0.14;
/// Out to here every cell stands its clump (m).
pub const GRASS_FULL_M: f32 = 8.0;
/// No grass past here (m): beyond it the land's own colour carries it.
pub const GRASS_REACH_M: f32 = 55.0;

/// What the grass's placement reads of the view (`grass_place.wgsl`'s `GrassView`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GrassView {
    /// The frustum in chart metres, inside where n · p + w ≥ 0.
    pub planes: [[f32; 4]; 6],
    /// The eye in chart metres; w: a pixel's angle (radians).
    pub eye: [f32; 4],
    /// The grid's first cell (x, z, in cells from the field's origin), its cells a side, a cell (m).
    pub grid: [f32; 4],
    /// The full-density distance (m), the reach (m), the room for clumps, 0.
    pub reach: [f32; 4],
}

impl GrassView {
    /// The view for an eye at chart point `eye` (m) over a field whose origin is `origin`, the frustum `planes`, a
    /// pixel spanning `pixel` radians: the grid about the eye, or none when the eye is too high over the land to see the
    /// grass (`ground` the land's highest point, m).
    pub fn about(
        eye: [f64; 3],
        origin: [f32; 2],
        planes: [[f32; 4]; 6],
        pixel: f32,
        ground: f32,
    ) -> Self {
        let reach = f64::from(GRASS_REACH_M);
        let cell = f64::from(GRASS_CELL_M);
        let corner =
            |eye: f64, origin: f32| ((eye - f64::from(origin) - reach) / cell).floor() as f32;
        let near = eye[1] < f64::from(ground) + reach;
        Self {
            planes,
            eye: [eye[0] as f32, eye[1] as f32, eye[2] as f32, pixel],
            grid: [
                corner(eye[0], origin[0]),
                corner(eye[2], origin[1]),
                if near {
                    (2.0 * reach / cell).ceil() as f32 + 1.0
                } else {
                    0.0
                },
                GRASS_CELL_M,
            ],
            reach: [GRASS_FULL_M, GRASS_REACH_M, GRASS_CAPACITY as f32, 0.0],
        }
    }

    /// Its grid's cells a side (0 for none).
    pub fn cells(&self) -> u32 {
        self.grid[2] as u32
    }
}

/// The grass's buffers and pipelines.
pub(crate) struct Grass {
    view: wgpu::Buffer,
    clumps: wgpu::Buffer,
    /// `DrawIndirectArgs`: a clump's vertices and the clumps the placement counted.
    args: wgpu::Buffer,
    place_group: wgpu::BindGroup,
    place: PipelineId,
    pipeline: PipelineId,
    /// Whether this frame placed any.
    placed: bool,
}

impl Grass {
    /// The placement reads the land's group (`terrain_layout`, group 1); the draw takes the land's groups 1-3
    /// (`layouts`) with the trees' defs.
    pub fn new(
        ctx: &mut RenderContext<'_>,
        terrain_layout: &wgpu::BindGroupLayout,
        layouts: Vec<wgpu::BindGroupLayout>,
        defs: ShaderDefs,
    ) -> Result<Self, ShaderError> {
        let device = ctx.device.clone();
        let view = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain grass view"),
            size: std::mem::size_of::<GrassView>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let clumps = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain grass clumps"),
            size: u64::from(GRASS_CAPACITY) * CLUMP_BYTES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let args = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain grass draw"),
            size: 16,
            usage: wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(
            &args,
            0,
            bytemuck::cast_slice(&[BLADES * BLADE_VERTICES, 0, 0, 0]),
        );
        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let place_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain grass placement"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
            ],
        });
        let place_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain grass placement"),
            layout: &place_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: clumps.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: args.as_entire_binding(),
                },
            ],
        });
        let place = ctx.add_pipeline(
            "terrain grass placement",
            "naval-terrain/shaders/grass_place.wgsl",
            defs.clone(),
            compute_recipe(vec![place_layout, terrain_layout.clone()], "place_grass"),
        )?;
        let attribute =
            |location: u32, format: wgpu::VertexFormat, offset: u64| wgpu::VertexAttribute {
                format,
                offset,
                shader_location: location,
            };
        let pipeline = ctx.add_scene_pipeline(
            "terrain grass",
            "naval-terrain/shaders/grass.wgsl",
            ScenePipeline {
                vertex_buffers: vec![VertexLayout {
                    stride: CLUMP_BYTES,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: vec![
                        attribute(0, wgpu::VertexFormat::Float32x4, 0),
                        attribute(1, wgpu::VertexFormat::Uint32x4, 16),
                    ],
                }],
                bind_group_layouts: layouts,
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    ..Default::default()
                },
                defs,
                ..ScenePipeline::new("vs_grass", "fs_grass")
            },
        )?;
        Ok(Self {
            view,
            clumps,
            args,
            place_group,
            place,
            pipeline,
            placed: false,
        })
    }

    /// Places this frame's grass for `view` (none when its grid is empty), reading the land's group.
    pub fn place(
        &mut self,
        ctx: &mut PrepareContext<'_>,
        terrain_group: &wgpu::BindGroup,
        view: &GrassView,
    ) {
        let cells = view.cells();
        self.placed = cells > 0;
        if !self.placed {
            return;
        }
        ctx.uploads
            .write_buffer(&self.view, 0, bytemuck::bytes_of(view));
        // The instance count back to 0; the vertex count stays.
        ctx.encoder.clear_buffer(&self.args, 4, Some(4));
        let mut pass = ctx
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("terrain grass placement"),
                timestamp_writes: ctx.timer.compute_pass("terrain grass placement"),
            });
        pass.set_pipeline(ctx.pipelines.compute(self.place));
        pass.set_bind_group(0, &self.place_group, &[]);
        pass.set_bind_group(1, terrain_group, &[]);
        pass.dispatch_workgroups(cells.div_ceil(8), cells.div_ceil(8), 1);
    }

    /// Draws the grass placed this frame. The caller has bound groups 1-3 as for the land.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &naval_render::PipelineRegistry,
    ) {
        if !self.placed {
            return;
        }
        pass.set_pipeline(pipelines.render(self.pipeline));
        pass.set_vertex_buffer(0, self.clumps.slice(..));
        pass.draw_indirect(&self.args, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shaders_share_the_grass_layout() {
        let draw = include_str!("../../shaders/grass.wgsl");
        assert!(draw.contains(&format!("const BLADES: u32 = {BLADES}u;")));
        assert!(draw.contains(&format!("const BLADE_VERTICES: u32 = {BLADE_VERTICES}u;")));
        // The first pair's left twice, four pairs up the blade and the tip three times: even, so every blade's first
        // triangle falls at the same parity of the strip.
        assert_eq!(BLADE_VERTICES, 1 + 4 * 2 + 3);
        assert_eq!(BLADE_VERTICES % 2, 0);
        // A clump's patch at full density.
        let patch: f32 = draw
            .split("const NEAR_PATCH_M: f32 = ")
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .and_then(|value| value.parse().ok())
            .unwrap();
        assert!((patch - GRASS_CELL_M * 0.75).abs() < 1e-6, "{patch}");
        assert_eq!(CLUMP_BYTES, 32);
        assert_eq!(std::mem::size_of::<GrassView>(), 4 * 4 * 9);
    }

    #[test]
    fn the_grid_follows_the_eye_and_leaves_the_sky_out() {
        let near = GrassView::about([1000.0, 12.0, -500.0], [0.0, 0.0], [[0.0; 4]; 6], 1e-3, 5.0);
        let cells = near.cells();
        assert!(cells as f32 * GRASS_CELL_M >= 2.0 * GRASS_REACH_M);
        // The grid's middle under the eye.
        let middle = near.grid[0] + cells as f32 * 0.5;
        assert!((middle * GRASS_CELL_M - 1000.0).abs() < GRASS_CELL_M * 2.0);
        let high = GrassView::about(
            [1000.0, 400.0, -500.0],
            [0.0, 0.0],
            [[0.0; 4]; 6],
            1e-3,
            5.0,
        );
        assert_eq!(high.cells(), 0);
    }
}
