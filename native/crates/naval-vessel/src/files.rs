//! Reading a setup file and its model from disk (native only: the rest of the crate reads bytes): the setup's `model`
//! is relative to the setup file, a `.gltf`'s buffers and images relative to it.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::inspect::{Inspection, inspect};
use crate::model::VesselModel;
use crate::setup::Setup;

/// A resolver for the files beside `model`: a URI relative to its folder, never above it.
pub fn beside(model: &Path) -> impl FnMut(&str) -> Result<Vec<u8>> + use<> {
    let folder = model.parent().map(Path::to_path_buf).unwrap_or_default();
    move |uri: &str| {
        let relative = Path::new(uri);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            bail!("{uri} lies outside the model's folder");
        }
        let path = folder.join(relative);
        std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
    }
}

/// The model a setup file names, as a path.
pub fn model_path(setup_path: &Path, setup: &Setup) -> PathBuf {
    setup_path
        .parent()
        .map_or_else(|| PathBuf::from(&setup.model), |dir| dir.join(&setup.model))
}

/// Reads the setup file at `path` and loads the model it names.
pub fn load(path: &Path) -> Result<VesselModel> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let setup = Setup::from_json(&bytes).with_context(|| format!("{}", path.display()))?;
    let model = model_path(path, &setup);
    let glb = std::fs::read(&model).with_context(|| format!("reading {}", model.display()))?;
    let name = model
        .file_stem()
        .map_or_else(|| "vessel".to_owned(), |s| s.to_string_lossy().into_owned());
    VesselModel::load(&setup, &name, &glb, &mut beside(&model))
        .with_context(|| format!("{}", model.display()))
}

/// Looks over the model at `path`.
pub fn inspect_file(path: &Path) -> Result<Inspection> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path
        .file_name()
        .map_or_else(|| "model".to_owned(), |s| s.to_string_lossy().into_owned());
    inspect(&name, &bytes, &mut beside(path)).with_context(|| format!("{}", path.display()))
}

/// Where a model's starter setup goes: `<model>.vessel.json` beside it.
pub fn setup_path(model: &Path) -> PathBuf {
    let stem = model
        .file_stem()
        .map_or_else(|| "model".to_owned(), |s| s.to_string_lossy().into_owned());
    model.with_file_name(format!("{stem}.vessel.json"))
}
