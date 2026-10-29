//! The land's shaders as the game ships them on every platform: composed as the land layer composes them (each style,
//! the High and Medium settings' defs, the trees' compute passes at level 0), validated by naga, and written out as
//! Metal (MSL), DirectX 12 (HLSL, shader model 6.0) and Vulkan (SPIR-V) code, which a Mac's test run never compiles.
//! The lamps at night (`lamps.wgsl`) with the High setting's defs, the only setting that draws them.
//!
//! What naga does not check is that no screen derivative is taken in non-uniform control flow (it lifts that rule for
//! fragment shaders): `surface.wgsl` takes none of its own (`tile_uv`, `surface_read`) and the map features take theirs
//! before any branch (`feature_site`), so its branches (the seabed, the classes a point holds, the style under a
//! photograph) are well defined on every backend. `no_read_takes_screen_derivatives_of_its_own` holds the files to it.

use naga::back::{hlsl, msl, spv};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naval_render::{ShaderDefValue, ShaderDefs, ShaderLibrary};

const STYLES: [&str; 4] = [
    "STYLE_TROPICAL",
    "STYLE_SNOW",
    "STYLE_VOLCANIC",
    "STYLE_CHALK",
];

fn library() -> ShaderLibrary {
    let mut library = ShaderLibrary::new();
    library
        .add(naval_sky::shaders::ENVIRONMENT_FILE)
        .expect("the environment module");
    library
        .add(naval_clouds::shaders::SHADOW_FILE)
        .expect("the clouds' shadow module");
    for file in [
        naval_render::shader_file!("shaders/bindings.wgsl"),
        naval_render::shader_file!("shaders/hex.wgsl"),
        naval_render::shader_file!("shaders/features.wgsl"),
        naval_render::shader_file!("shaders/night.wgsl"),
        naval_render::shader_file!("shaders/lamps.wgsl"),
        naval_render::shader_file!("shaders/materials.wgsl"),
        naval_render::shader_file!("shaders/surface.wgsl"),
        naval_render::shader_file!("shaders/lighting.wgsl"),
        naval_render::shader_file!("shaders/structures.wgsl"),
        naval_render::shader_file!("shaders/cells.wgsl"),
        naval_render::shader_file!("shaders/forest.wgsl"),
        naval_render::shader_file!("shaders/terrain.wgsl"),
        naval_render::shader_file!("shaders/caster.wgsl"),
        naval_render::shader_file!("shaders/impostor.wgsl"),
        naval_render::shader_file!("shaders/foliage.wgsl"),
        naval_render::shader_file!("shaders/tree_wind.wgsl"),
        naval_render::shader_file!("shaders/trees.wgsl"),
        naval_render::shader_file!("shaders/near_trees.wgsl"),
        naval_render::shader_file!("shaders/tree_near_bin.wgsl"),
        naval_render::shader_file!("shaders/tree_place.wgsl"),
        naval_render::shader_file!("shaders/tree_caster.wgsl"),
        naval_render::shader_file!("shaders/tree_shade.wgsl"),
        naval_render::shader_file!("shaders/tree_surface.wgsl"),
        naval_render::shader_file!("shaders/tree_bake.wgsl"),
        naval_render::shader_file!("shaders/tree_resolve.wgsl"),
        naval_render::shader_file!("shaders/tree_mips.wgsl"),
        naval_render::shader_file!("shaders/grass_place.wgsl"),
        naval_render::shader_file!("shaders/grass.wgsl"),
        naval_render::shader_file!("shaders/buildings.wgsl"),
        naval_render::shader_file!("shaders/buildings_caster.wgsl"),
        naval_render::shader_file!("shaders/horizon.wgsl"),
        naval_render::shader_file!("shaders/infra.wgsl"),
    ] {
        library.add(file).expect("the land's shaders register");
    }
    library
}

fn defs(names: &[&str]) -> ShaderDefs {
    names
        .iter()
        .map(|n| ((*n).to_owned(), ShaderDefValue::Bool(true)))
        .collect()
}

/// Validates `module` and writes it out for each backend; panics with the backend and the reason.
fn translate(name: &str, module: &naga::Module) {
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .unwrap_or_else(|e| panic!("{name}: naga's validation: {e:?}"));
    let options = msl::Options {
        lang_version: (2, 4),
        ..msl::Options::default()
    };
    msl::write_string(module, &info, &options, &msl::PipelineOptions::default())
        .unwrap_or_else(|e| panic!("{name}: MSL: {e:?}"));
    let mut text = String::new();
    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V6_0,
        ..hlsl::Options::default()
    };
    hlsl::Writer::new(&mut text, &options, &hlsl::PipelineOptions::default())
        .write(module, &info, None)
        .unwrap_or_else(|e| panic!("{name}: HLSL: {e:?}"));
    spv::write_vec(module, &info, &spv::Options::default(), None)
        .unwrap_or_else(|e| panic!("{name}: SPIR-V: {e:?}"));
}

#[test]
fn the_lands_shaders_translate_for_metal_directx_and_vulkan() {
    let mut library = library();
    for style in STYLES {
        let high = [
            style,
            "DETAILED",
            "TREES",
            "FEATURES",
            "MATERIALS",
            "RELIEF",
        ];
        for (setting, names) in [("high", &high[..]), ("medium", &[style][..])] {
            for path in ["terrain.wgsl", "caster.wgsl"] {
                let module = library
                    .compose(&format!("naval-terrain/shaders/{path}"), &defs(names))
                    .unwrap_or_else(|e| panic!("{path} ({style}, {setting}): {}", e.message));
                translate(&format!("{path} ({style}, {setting})"), &module);
            }
        }
        let module = library
            .compose("naval-terrain/shaders/lamps.wgsl", &defs(&high))
            .unwrap_or_else(|e| panic!("lamps.wgsl ({style}): {}", e.message));
        translate(&format!("lamps.wgsl ({style})"), &module);
        // The infrastructure's (roads, bridges, a generated base's pieces).
        let module = library
            .compose("naval-terrain/shaders/infra.wgsl", &defs(&[style]))
            .unwrap_or_else(|e| panic!("infra.wgsl ({style}): {}", e.message));
        translate(&format!("infra.wgsl ({style})"), &module);
        let trees = [
            style,
            "DETAILED",
            "TREES",
            "FEATURES",
            "MATERIALS",
            "RELIEF",
            "LOD_ZERO",
        ];
        for path in [
            "trees.wgsl",
            "near_trees.wgsl",
            "tree_near_bin.wgsl",
            "tree_place.wgsl",
            "tree_caster.wgsl",
            "tree_shade.wgsl",
            "grass_place.wgsl",
            "grass.wgsl",
        ] {
            let module = library
                .compose(&format!("naval-terrain/shaders/{path}"), &defs(&trees))
                .unwrap_or_else(|e| panic!("{path} ({style}): {}", e.message));
            translate(&format!("{path} ({style})"), &module);
        }
    }
    // The buildings: High and Ultra's facade textures and clutter caster, Medium's drawn windows, each with the fading
    // chunks' variant.
    for (path, names) in [
        ("buildings.wgsl", &["FACADE_TEXTURES"][..]),
        ("buildings.wgsl", &[][..]),
        ("buildings.wgsl", &["FACADE_TEXTURES", "BUILDINGS_FADE"][..]),
        ("buildings.wgsl", &["BUILDINGS_FADE"][..]),
        ("buildings_caster.wgsl", &["CLUTTER_CASTER"][..]),
        ("buildings_caster.wgsl", &[][..]),
    ] {
        let module = library
            .compose(&format!("naval-terrain/shaders/{path}"), &defs(names))
            .unwrap_or_else(|e| panic!("{path} ({names:?}): {}", e.message));
        translate(&format!("{path} ({names:?})"), &module);
    }
    // The trees' impostor bake, once a device, with no defs.
    for path in ["tree_bake.wgsl", "tree_resolve.wgsl", "tree_mips.wgsl"] {
        let module = library
            .compose(&format!("naval-terrain/shaders/{path}"), &defs(&[]))
            .unwrap_or_else(|e| panic!("{path}: {}", e.message));
        translate(path, &module);
    }
    // The horizon ring (High and Ultra), with no defs.
    let module = library
        .compose("naval-terrain/shaders/horizon.wgsl", &defs(&[]))
        .unwrap_or_else(|e| panic!("horizon.wgsl: {}", e.message));
    translate("horizon.wgsl", &module);
}

#[test]
fn no_read_takes_screen_derivatives_of_its_own() {
    // `surface.wgsl` and the modules it calls run in branches of the land's fragment stage: an implicit-level read or a
    // derivative there would be undefined where neighbouring pixels take different branches.
    for (path, text) in [
        ("surface.wgsl", include_str!("../shaders/surface.wgsl")),
        ("materials.wgsl", include_str!("../shaders/materials.wgsl")),
        ("hex.wgsl", include_str!("../shaders/hex.wgsl")),
        ("night.wgsl", include_str!("../shaders/night.wgsl")),
        // The fine relief's reads (`relief_height`, `relief_gradient`) run in branches too.
        ("bindings.wgsl", include_str!("../shaders/bindings.wgsl")),
    ] {
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for call in [
                "textureSample(",
                "textureSampleBias(",
                "dpdx(",
                "dpdy(",
                "fwidth(",
            ] {
                assert!(
                    !code.contains(call),
                    "{path}:{}: `{call}` takes screen derivatives: {line}",
                    n + 1
                );
            }
        }
    }
    // The features take theirs in `feature_site` alone, which the fragment stage calls before any branch.
    let features = include_str!("../shaders/features.wgsl");
    let derivatives = features
        .lines()
        .filter(|l| l.split("//").next().unwrap_or("").contains("dpdx("))
        .count();
    assert_eq!(
        derivatives, 1,
        "features.wgsl takes screen derivatives in `feature_px` alone"
    );
    let terrain = include_str!("../shaders/terrain.wgsl");
    let site = terrain
        .find("feature_site(chart")
        .expect("terrain.wgsl takes the features' site");
    let branch = terrain
        .find("if height < SEABED_PLAIN_M")
        .expect("terrain.wgsl's seabed branch");
    assert!(
        site < branch,
        "the features' site is taken before the seabed's branch"
    );
}
