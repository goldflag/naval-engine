//! The ocean's WGSL, composed by the renderer's shader library ([`naval_render::ShaderLibrary`]) as every crate's WGSL
//! is: the composable modules (the renderer core's `naval::camera`, `naval::shadow`, `naval::motion` and
//! `naval::lights`, the clouds' `naval_clouds::shadow`, `naval_ocean::noise`, `naval_ocean::reflection`,
//! `naval_ocean::screen_reflection`, `naval_ocean::water_body`, `naval_ocean::foam`, `naval_ocean::point_lights`,
//! `naval_ocean::mx_noise`, `naval_ocean::wake`, `naval_ocean::oil`, `naval_ocean::rain`, `naval_ocean::spray_haze`,
//! and the sky's `naval_sky::environment`) and the entry files that import them.

use std::borrow::Cow;

use naval_render::{ShaderDefValue, ShaderError, ShaderLibrary};

/// An entry file: compute or render stages.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub path: &'static str,
    pub source: &'static str,
}

pub const FFT: Entry = Entry {
    path: "naval-ocean/shaders/fft.wgsl",
    source: include_str!("../shaders/fft.wgsl"),
};
pub const RESOLVE: Entry = Entry {
    path: "naval-ocean/shaders/resolve.wgsl",
    source: include_str!("../shaders/resolve.wgsl"),
};
pub const MIPS: Entry = Entry {
    path: "naval-ocean/shaders/mips.wgsl",
    source: include_str!("../shaders/mips.wgsl"),
};
pub const SURFACE: Entry = Entry {
    path: "naval-ocean/shaders/surface.wgsl",
    source: include_str!("../shaders/surface.wgsl"),
};
pub const ENVIRONMENT_FILL: Entry = Entry {
    path: "naval-ocean/shaders/environment_fill.wgsl",
    source: include_str!("../shaders/environment_fill.wgsl"),
};
pub const BACKDROP: Entry = Entry {
    path: "naval-ocean/shaders/backdrop.wgsl",
    source: include_str!("../shaders/backdrop.wgsl"),
};
pub const STAMPS: Entry = Entry {
    path: "naval-ocean/shaders/stamps.wgsl",
    source: include_str!("../shaders/stamps.wgsl"),
};
pub const UNDERWATER_PROBE: Entry = Entry {
    path: "naval-ocean/shaders/underwater_probe.wgsl",
    source: include_str!("../shaders/underwater_probe.wgsl"),
};
pub const HULL_HEIGHTS: Entry = Entry {
    path: "naval-ocean/shaders/hull_heights.wgsl",
    source: include_str!("../shaders/hull_heights.wgsl"),
};
pub const UNDERWATER: Entry = Entry {
    path: "naval-ocean/shaders/underwater.wgsl",
    source: include_str!("../shaders/underwater.wgsl"),
};
pub const UNDERWATER_MOTES: Entry = Entry {
    path: "naval-ocean/shaders/underwater_motes.wgsl",
    source: include_str!("../shaders/underwater_motes.wgsl"),
};
pub const UNDERWATER_BUBBLES: Entry = Entry {
    path: "naval-ocean/shaders/underwater_bubbles.wgsl",
    source: include_str!("../shaders/underwater_bubbles.wgsl"),
};
pub const REFLECTION_TILES: Entry = Entry {
    path: "naval-ocean/shaders/reflection_tiles.wgsl",
    source: include_str!("../shaders/reflection_tiles.wgsl"),
};
pub const WAKE_FIELD: Entry = Entry {
    path: "naval-ocean/shaders/wake_field.wgsl",
    source: include_str!("../shaders/wake_field.wgsl"),
};
pub const FOAM_SIM: Entry = Entry {
    path: "naval-ocean/shaders/foam_sim.wgsl",
    source: include_str!("../shaders/foam_sim.wgsl"),
};
pub const RAIN_HEIGHTS: Entry = Entry {
    path: "naval-ocean/shaders/rain_heights.wgsl",
    source: include_str!("../shaders/rain_heights.wgsl"),
};
pub const CAUSTICS: Entry = Entry {
    path: "naval-ocean/shaders/caustics.wgsl",
    source: include_str!("../shaders/caustics.wgsl"),
};
pub const SPINDRIFT: Entry = Entry {
    path: "naval-ocean/shaders/spindrift.wgsl",
    source: include_str!("../shaders/spindrift.wgsl"),
};

/// Every file of the ocean's own that its pipelines are built from, with its path on disk (hot reload), for the
/// renderer's library: the modules in dependency order, then the entries. The stand-in environment's fill and the
/// backdrop (tools only) are not among them, nor the clouds' and the sky's modules ([`EXTERNAL_FILES`]).
pub const FILES: [naval_render::ShaderFile; 32] = [
    naval_render::shader_file!("shaders/water_properties.wgsl"),
    naval_render::shader_file!("shaders/pcg.wgsl"),
    naval_render::shader_file!("shaders/complex.wgsl"),
    naval_render::shader_file!("shaders/noise.wgsl"),
    naval_render::shader_file!("shaders/reflection.wgsl"),
    naval_render::shader_file!("shaders/water_body.wgsl"),
    naval_render::shader_file!("shaders/foam.wgsl"),
    naval_render::shader_file!("shaders/point_lights.wgsl"),
    naval_render::shader_file!("shaders/mx_noise.wgsl"),
    naval_render::shader_file!("shaders/wake.wgsl"),
    naval_render::shader_file!("shaders/oil.wgsl"),
    naval_render::shader_file!("shaders/waves.wgsl"),
    naval_render::shader_file!("shaders/screen_reflection.wgsl"),
    naval_render::shader_file!("shaders/underwater_ray.wgsl"),
    naval_render::shader_file!("shaders/rain.wgsl"),
    naval_render::shader_file!("shaders/spray_haze.wgsl"),
    naval_render::shader_file!("shaders/underwater_probe.wgsl"),
    naval_render::shader_file!("shaders/hull_heights.wgsl"),
    naval_render::shader_file!("shaders/rain_heights.wgsl"),
    naval_render::shader_file!("shaders/caustics.wgsl"),
    naval_render::shader_file!("shaders/underwater.wgsl"),
    naval_render::shader_file!("shaders/underwater_motes.wgsl"),
    naval_render::shader_file!("shaders/underwater_bubbles.wgsl"),
    naval_render::shader_file!("shaders/fft.wgsl"),
    naval_render::shader_file!("shaders/resolve.wgsl"),
    naval_render::shader_file!("shaders/mips.wgsl"),
    naval_render::shader_file!("shaders/surface.wgsl"),
    naval_render::shader_file!("shaders/stamps.wgsl"),
    naval_render::shader_file!("shaders/wake_field.wgsl"),
    naval_render::shader_file!("shaders/foam_sim.wgsl"),
    naval_render::shader_file!("shaders/spindrift.wgsl"),
    naval_render::shader_file!("shaders/reflection_tiles.wgsl"),
];

/// Other crates' modules the ocean's files import: the clouds' shadow functions (the surface) and the sky's environment
/// (the surface, the reflection, the spindrift and the backdrop).
pub const EXTERNAL_FILES: [naval_render::ShaderFile; 2] = [
    naval_clouds::shaders::SHADOW_FILE,
    naval_sky::shaders::ENVIRONMENT_FILE,
];

/// Registers the ocean's files with the renderer's library (and [`EXTERNAL_FILES`], unless another layer has), so its
/// pipelines can be built on the registry and hot-reload.
pub fn register(
    ctx: &mut naval_render::RenderContext<'_>,
) -> Result<(), naval_render::ShaderError> {
    for file in EXTERNAL_FILES {
        if ctx.library.text(file.path).is_none() {
            ctx.add_shader(file)?;
        }
    }
    for file in FILES {
        ctx.add_shader(file)?;
    }
    Ok(())
}

/// Integer shader defs as the library takes them.
pub fn library_defs(defs: &[(&str, u32)]) -> naval_render::ShaderDefs {
    defs.iter()
        .map(|(name, value)| (name.to_string(), ShaderDefValue::UInt(*value)))
        .collect()
}

/// The renderer's shader library holding the ocean's files ([`FILES`]) and the modules they import from the clouds and
/// the sky ([`EXTERNAL_FILES`]): what a pipeline built outside a renderer's registry composes with
/// ([`naval_render::MakePipelines::Now`], tools, tests). A renderer's library holds the same files ([`register`]).
pub fn library() -> ShaderLibrary {
    let mut library = ShaderLibrary::new();
    for file in EXTERNAL_FILES.into_iter().chain(FILES) {
        library
            .add(file)
            .expect("registering a file composes nothing");
    }
    library
}

/// Compose `entry` into a naga module in the ocean's [`library`] with integer shader defs, e.g. `("CASCADES", 3)`.
pub fn naga_module(entry: Entry, defs: &[(&str, u32)]) -> Result<wgpu::naga::Module, ShaderError> {
    let mut library = library();
    library.add_source(entry.path, entry.source)?;
    library.compose(entry.path, &library_defs(defs))
}

/// Compose `entry` and create the wgpu shader module; a composition error names the file and line.
pub fn shader_module(
    device: &wgpu::Device,
    entry: Entry,
    defs: &[(&str, u32)],
) -> wgpu::ShaderModule {
    let module = naga_module(entry, defs).unwrap_or_else(|error| panic!("composing {error}"));
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(entry.path),
        source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every entry composes and validates without a GPU.
    #[test]
    fn every_shader_composes_and_validates() {
        for entry in [
            FFT,
            RESOLVE,
            MIPS,
            SURFACE,
            ENVIRONMENT_FILL,
            BACKDROP,
            WAKE_FIELD,
            FOAM_SIM,
            STAMPS,
            UNDERWATER_PROBE,
            HULL_HEIGHTS,
            UNDERWATER,
            UNDERWATER_MOTES,
            UNDERWATER_BUBBLES,
            SPINDRIFT,
            RAIN_HEIGHTS,
            CAUSTICS,
            REFLECTION_TILES,
        ] {
            // The surface also with the scene's point lights and the oil slicks compiled in.
            let lit: &[&[(&str, u32)]] = if entry.path == SURFACE.path {
                &[
                    &[],
                    &[("POINT_LIGHTS", 1)],
                    &[("OIL", 1)],
                    &[("POINT_LIGHTS", 1), ("OIL", 1)],
                ]
            } else {
                &[&[]]
            };
            for extra in lit {
                let defs: Vec<(&str, u32)> = [("CASCADES", 3), ("REFLECTION_STEPS", 16)]
                    .into_iter()
                    .chain(extra.iter().copied())
                    .collect();
                let module = naga_module(entry, &defs).unwrap_or_else(|error| panic!("{error}"));
                wgpu::naga::valid::Validator::new(
                    wgpu::naga::valid::ValidationFlags::all(),
                    wgpu::naga::valid::Capabilities::default(),
                )
                .validate(&module)
                .unwrap_or_else(|error| panic!("{}: {error:?}", entry.path));
            }
        }
    }
}
