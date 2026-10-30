//! Ships from any glTF (`--ship <setup.json>`, naval-vessel, as many as are named): each model and its setup read from
//! disk, drawn where `crate::afloat` has her on the sea (her pose afloat through `crate::join::pose`), lit as the rest
//! of the scene.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use naval_clouds::GameSky;
use naval_render::Renderer;
use naval_vessel::{ModelId, Vessel, VesselInstance, VesselLayer, VesselLighting, VesselPose};

pub struct Ships {
    pub layer: VesselLayer,
    /// Each ship's model and what her setup resolved to, in the order named.
    pub models: Vec<(ModelId, Vessel)>,
}

impl Ships {
    /// The models `setups` name, uploaded.
    pub fn new(renderer: &mut Renderer, sky: &GameSky, setups: &[PathBuf]) -> Result<Self> {
        let mut ctx = renderer.context();
        let mut layer = VesselLayer::new(
            &mut ctx,
            sky.sky.environment(),
            VesselLighting::daylight(sky.light()),
        )
        .map_err(|e| anyhow!("the ships' shaders: {e:?}"))?;
        let mut models = Vec::new();
        for setup in setups {
            let model = naval_vessel::files::load(setup)?;
            for note in &model.notes {
                println!("{}: {note}", setup.display());
            }
            let v = &model.vessel;
            println!(
                "{}: {:.1} m long, {:.1} m beam, {:.1} m draft, {} triangles, {} textures",
                v.name,
                v.length,
                v.beam,
                v.draft,
                model.triangles(),
                model.textures.len()
            );
            let id = layer.add_model(ctx.device, ctx.queue, &model);
            models.push((id, model.vessel));
        }
        Ok(Self { layer, models })
    }

    /// Draw each at her pose this frame, in the order named.
    pub fn place(&mut self, poses: impl IntoIterator<Item = VesselPose>) {
        self.layer.instances = self
            .models
            .iter()
            .zip(poses)
            .enumerate()
            .map(|(key, (&(model, _), pose))| VesselInstance {
                model,
                transform: pose.transform(),
                key: key as u64,
                lamps: true,
            })
            .collect();
    }

    /// This frame's light from the sky, a lightning `flash` under the scene's `ambient`, and its haze.
    pub fn advance(
        &mut self,
        sky: &GameSky,
        ambient: f32,
        flash: f32,
        fog: naval_render::SceneFog,
    ) {
        self.layer.lighting.follow(&sky.sky, ambient, flash);
        self.layer.lighting.fog = fog;
    }
}
