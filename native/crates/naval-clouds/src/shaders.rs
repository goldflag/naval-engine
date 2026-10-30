//! The clouds' WGSL, composed over `naval-sky`'s modules by the renderer's shader library
//! ([`naval_render::ShaderLibrary`]) as every crate's WGSL is: the shared modules (`naval_clouds::common`, `field`,
//! `march`, `hooks`, and the passes' `screen` and `precipitation`) and the passes that import them.

use std::borrow::Cow;

use naval_render::{ShaderDefValue, ShaderLibrary};

pub const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
pub const FIELD_WGSL: &str = include_str!("../shaders/field.wgsl");
pub const MARCH_WGSL: &str = include_str!("../shaders/march.wgsl");
pub const HOOKS_WGSL: &str = include_str!("../shaders/hooks.wgsl");
/// Consumers' cloud shadow module (`naval_clouds::shadow`); see [`SHADOW_FILE`].
pub const SHADOW_WGSL: &str = include_str!("../shaders/shadow.wgsl");
/// The clouds' shadow functions (`naval_clouds::shadow`) as a library file, for the layers that import them (the
/// ocean, the ships, the land) to register with the renderer's library.
pub const SHADOW_FILE: naval_render::ShaderFile = naval_render::shader_file!("shaders/shadow.wgsl");

pub const NOISE_WGSL: &str = include_str!("../shaders/noise.wgsl");
pub const DETAIL_NOISE_WGSL: &str = include_str!("../shaders/detail_noise.wgsl");
pub const SHADOW_PASS_WGSL: &str = include_str!("../shaders/shadow_pass.wgsl");
pub const SHADOW_RESOLVE_WGSL: &str = include_str!("../shaders/shadow_resolve.wgsl");
pub const GROUND_WGSL: &str = include_str!("../shaders/ground.wgsl");
pub const MARCH_PASS_WGSL: &str = include_str!("../shaders/march_pass.wgsl");
pub const RESOLVE_WGSL: &str = include_str!("../shaders/resolve.wgsl");
pub const COMPOSITE_WGSL: &str = include_str!("../shaders/composite.wgsl");
pub const RAIN_WGSL: &str = include_str!("../shaders/rain.wgsl");
pub const SPLASHES_WGSL: &str = include_str!("../shaders/splashes.wgsl");
pub const BOLT_WGSL: &str = include_str!("../shaders/bolt.wgsl");
pub const AERIAL_WGSL: &str = include_str!("../shaders/aerial.wgsl");
pub const GLOW_FIELD_WGSL: &str = include_str!("../shaders/glow_field.wgsl");
pub const WEATHER_WGSL: &str = include_str!("../shaders/weather.wgsl");

/// The clouds' WGSL for the renderer core's shader library (with the files' disk paths, for hot reload): the shared
/// modules in dependency order (over the sky's, `naval_sky::shaders::FILES`), then the passes. [`register`] adds
/// them; the keys match the paths the clouds compose with here.
pub const FILES: [naval_render::ShaderFile; 20] = [
    naval_render::shader_file!("shaders/common.wgsl"),
    naval_render::shader_file!("shaders/screen.wgsl"),
    naval_render::shader_file!("shaders/precipitation.wgsl"),
    naval_render::shader_file!("shaders/field.wgsl"),
    naval_render::shader_file!("shaders/march.wgsl"),
    naval_render::shader_file!("shaders/hooks.wgsl"),
    naval_render::shader_file!("shaders/noise.wgsl"),
    naval_render::shader_file!("shaders/detail_noise.wgsl"),
    naval_render::shader_file!("shaders/shadow_pass.wgsl"),
    naval_render::shader_file!("shaders/shadow_resolve.wgsl"),
    naval_render::shader_file!("shaders/ground.wgsl"),
    naval_render::shader_file!("shaders/march_pass.wgsl"),
    naval_render::shader_file!("shaders/resolve.wgsl"),
    naval_render::shader_file!("shaders/composite.wgsl"),
    naval_render::shader_file!("shaders/rain.wgsl"),
    naval_render::shader_file!("shaders/splashes.wgsl"),
    naval_render::shader_file!("shaders/bolt.wgsl"),
    naval_render::shader_file!("shaders/aerial.wgsl"),
    naval_render::shader_file!("shaders/weather.wgsl"),
    naval_render::shader_file!("shaders/glow_field.wgsl"),
];

/// Library keys of the passes in [`FILES`].
pub const NOISE_PATH: &str = "naval-clouds/shaders/noise.wgsl";
pub const DETAIL_NOISE_PATH: &str = "naval-clouds/shaders/detail_noise.wgsl";
pub const SHADOW_PASS_PATH: &str = "naval-clouds/shaders/shadow_pass.wgsl";
pub const SHADOW_RESOLVE_PATH: &str = "naval-clouds/shaders/shadow_resolve.wgsl";
pub const GROUND_PATH: &str = "naval-clouds/shaders/ground.wgsl";
pub const MARCH_PASS_PATH: &str = "naval-clouds/shaders/march_pass.wgsl";
pub const RESOLVE_PATH: &str = "naval-clouds/shaders/resolve.wgsl";
pub const COMPOSITE_PATH: &str = "naval-clouds/shaders/composite.wgsl";
pub const RAIN_PATH: &str = "naval-clouds/shaders/rain.wgsl";
pub const SPLASHES_PATH: &str = "naval-clouds/shaders/splashes.wgsl";
pub const BOLT_PATH: &str = "naval-clouds/shaders/bolt.wgsl";
pub const AERIAL_PATH: &str = "naval-clouds/shaders/aerial.wgsl";
pub const GLOW_FIELD_PATH: &str = "naval-clouds/shaders/glow_field.wgsl";
pub const WEATHER_PATH: &str = "naval-clouds/shaders/weather.wgsl";

/// Adds the clouds' WGSL to the renderer core's library, after the sky's (`naval_sky::shaders::register`).
pub fn register(
    ctx: &mut naval_render::RenderContext<'_>,
) -> Result<(), naval_render::ShaderError> {
    // The aerial perspective reads the clouds' ground shadow through the consumers' module, which the layers that
    // import it register when absent too.
    if ctx.library.text(SHADOW_FILE.path).is_none() {
        ctx.add_shader(SHADOW_FILE)?;
    }
    for file in FILES {
        ctx.add_shader(file)?;
    }
    Ok(())
}

/// The clouds' shared modules in dependency order (file path, source): what [`crate::layer::CloudLayer::hooks`]
/// hands the sky and every cloud pass composes over.
pub fn modules() -> Vec<(String, String)> {
    [
        ("naval-clouds/shaders/shadow.wgsl", SHADOW_WGSL),
        ("naval-clouds/shaders/common.wgsl", COMMON_WGSL),
        ("naval-clouds/shaders/field.wgsl", FIELD_WGSL),
        ("naval-clouds/shaders/march.wgsl", MARCH_WGSL),
        ("naval-clouds/shaders/hooks.wgsl", HOOKS_WGSL),
    ]
    .into_iter()
    .map(|(path, source)| (path.to_owned(), source.to_owned()))
    .collect()
}

/// The renderer's shader library holding the sky's files and the clouds' ([`SHADOW_FILE`], [`FILES`]): what a pass
/// built outside a renderer's registry composes with. A renderer's library holds the same files
/// (`naval_sky::shaders::register`, [`register`]).
pub fn library() -> ShaderLibrary {
    let mut library = naval_sky::shaders::library();
    for file in std::iter::once(SHADOW_FILE).chain(FILES) {
        library
            .add(file)
            .expect("registering a file composes nothing");
    }
    library
}

/// A pass's naga module, composed in the clouds' [`library`] with shader defs.
pub fn compose(
    source: &str,
    file_path: &str,
    defs: &[(&str, ShaderDefValue)],
) -> Result<wgpu::naga::Module, String> {
    let mut library = library();
    library
        .add_source(file_path, source)
        .map_err(|error| error.to_string())?;
    let defs = defs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
    library
        .compose(file_path, &defs)
        .map_err(|error| error.to_string())
}

/// A pass's shader module. Panics with the library's report if the clouds' own WGSL does not compose: a bug in this
/// crate, caught by its tests.
pub fn shader_module(
    device: &wgpu::Device,
    source: &str,
    file_path: &str,
    defs: &[(&str, ShaderDefValue)],
) -> wgpu::ShaderModule {
    let module = compose(source, file_path, defs).unwrap_or_else(|report| panic!("{report}"));
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(file_path),
        source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(module: &wgpu::naga::Module, path: &str) {
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::default(),
        )
        .validate(module)
        .unwrap_or_else(|error| panic!("{path}: {error:?}"));
    }

    #[test]
    fn every_pass_composes_and_validates() {
        for (source, path) in [
            (NOISE_WGSL, NOISE_PATH),
            (DETAIL_NOISE_WGSL, DETAIL_NOISE_PATH),
            (SHADOW_PASS_WGSL, SHADOW_PASS_PATH),
            (SHADOW_RESOLVE_WGSL, SHADOW_RESOLVE_PATH),
            (GROUND_WGSL, GROUND_PATH),
            (MARCH_PASS_WGSL, MARCH_PASS_PATH),
            (RESOLVE_WGSL, RESOLVE_PATH),
            (COMPOSITE_WGSL, COMPOSITE_PATH),
            (RAIN_WGSL, RAIN_PATH),
            (SPLASHES_WGSL, SPLASHES_PATH),
            (BOLT_WGSL, BOLT_PATH),
            (WEATHER_WGSL, WEATHER_PATH),
            (AERIAL_WGSL, AERIAL_PATH),
            (GLOW_FIELD_WGSL, GLOW_FIELD_PATH),
        ] {
            let module =
                compose(source, path, &[]).unwrap_or_else(|report| panic!("{path}: {report}"));
            validate(&module, path);
        }
    }

    /// The sky's dome and bake compose over the hooks, as `Sky::attach_clouds` builds them.
    #[test]
    fn the_sky_composes_over_the_hooks() {
        let defs = [("CLOUDS", ShaderDefValue::Bool(true))];
        for (source, path) in [
            (naval_sky::shaders::DOME_WGSL, "naval-sky/shaders/dome.wgsl"),
            (
                naval_sky::shaders::ENVIRONMENT_BAKE_WGSL,
                "naval-sky/shaders/environment_bake.wgsl",
            ),
        ] {
            let module = naval_sky::shaders::compose_with(source, path, &modules(), &defs)
                .unwrap_or_else(|report| panic!("{path}: {report}"));
            validate(&module, path);
        }
    }

    #[test]
    fn the_shadow_module_composes_into_a_consumer() {
        let mut library = ShaderLibrary::new();
        library.add(SHADOW_FILE).expect("shadow module");
        let consumer = r#"
#import naval_clouds::shadow::{CloudShadow, cloud_shadow, cloud_shadow_fast}
@group(3) @binding(4) var shadow_map: texture_2d<f32>;
@group(3) @binding(5) var shadow_sampler: sampler;
@group(3) @binding(6) var<uniform> shadow: CloudShadow;
@fragment
fn main(@location(0) world: vec3<f32>) -> @location(0) vec4<f32> {
    let lit = cloud_shadow(shadow_map, shadow_sampler, shadow, world) * cloud_shadow_fast(shadow_map, shadow_sampler, shadow, world);
    return vec4<f32>(vec3<f32>(lit), 1.0);
}
"#;
        library
            .add_source("consumer.wgsl", consumer)
            .expect("consumer registers");
        let module = library
            .compose("consumer.wgsl", &Vec::new())
            .expect("consumer composes");
        validate(&module, "consumer");
    }
}
