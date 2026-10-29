//! Strike debris in the renderer's sun shadow maps: the shards drawn depth only into every map, as the scene draws
//! them (`shaders/debris.wgsl`'s `caster_vertex`). Smoke is not a depth caster: it goes into the renderer's soft
//! occluders' map, which `crate::shadow` draws (`naval_render::SunOccluders`).

use naval_render::{CasterPipeline, PipelineId, RenderContext, ShaderError};

/// The debris casters' pipeline.
pub(crate) struct SunCasters {
    debris_pipeline: PipelineId,
}

impl SunCasters {
    /// Builds the debris casters' pipeline from `debris_path` (its `caster_vertex`, reading the debris batch at
    /// group 2 behind the lighting at group 1).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        debris_path: &str,
        lighting_layout: &wgpu::BindGroupLayout,
        debris_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, ShaderError> {
        let debris_pipeline = ctx.add_caster_pipeline(
            "strike debris casters",
            debris_path,
            CasterPipeline {
                bind_group_layouts: vec![lighting_layout.clone(), debris_layout.clone()],
                ..CasterPipeline::new("caster_vertex")
            },
        )?;
        Ok(Self { debris_pipeline })
    }

    /// Records the debris (the batch's group and its count of pieces) into one sun map's pass.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &naval_render::PipelineRegistry,
        lighting: &wgpu::BindGroup,
        debris: &wgpu::BindGroup,
        count: u32,
    ) {
        // Each piece up to four boxes of 12 triangles, as the scene draws it.
        pass.set_pipeline(pipelines.render(self.debris_pipeline));
        pass.set_bind_group(1, lighting, &[]);
        pass.set_bind_group(2, debris, &[]);
        pass.draw(0..crate::debris::PIECE_VERTICES, 0..count);
    }
}
