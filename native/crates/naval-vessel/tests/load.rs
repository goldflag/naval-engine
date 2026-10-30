//! Loading made-up ships: glTF's own frame, a Z-up model in centimetres with its origin off her middle, a model with no
//! textures, normals or UVs, blended and cut-out windows and textures with KHR_texture_transform; the inspection's
//! guess at her axes; the files refused, each with its reason; malformed files (a few bytes of a plain ship changed)
//! refused rather than panicked or hung on; each map's own sampler, and a cut-out's coverage in its mips alone.

use glam::Vec3;
use naval_vessel::read::{Alpha, Filter, Sampling, Slot, Wrap};
use naval_vessel::setup::{Axis, FunnelSetup, LampKind, LampSetup};
use naval_vessel::{Setup, VesselModel, inspect, no_files};
use serde_json::json;

use crate::fixture::{self, BEAM, DRAFT, FREEBOARD, Glb, HOUSE, LENGTH, Shape, paint};

fn load(setup: &Setup, glb: &Glb) -> anyhow::Result<VesselModel> {
    VesselModel::load(setup, "fixture", &glb.bytes(), &mut no_files)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3 * b.abs().max(1.0)
}

#[test]
fn a_gltf_ship_resolves_to_her_size() {
    let setup = Setup {
        draft: Some(DRAFT),
        funnels: vec![FunnelSetup {
            name: "funnel".into(),
            at: [0.0, 14.0, 2.0],
            smoke: 0.7,
            radius: Some(1.5),
        }],
        lights: vec![LampSetup {
            name: "masthead".into(),
            at: [0.0, 26.0, -3.7],
            kind: LampKind::Masthead,
            color: None,
            intensity: None,
        }],
        length: Some(LENGTH),
        ..Setup::new("plain.glb")
    };
    let model = load(&setup, &fixture::plain()).unwrap();
    let v = &model.vessel;
    assert!(close(v.length, LENGTH), "{}", v.length);
    assert!(close(v.beam, BEAM), "{}", v.beam);
    assert!(close(v.draft, DRAFT), "{}", v.draft);
    assert!(
        close(v.air_draft, FREEBOARD + HOUSE + 16.0),
        "{}",
        v.air_draft
    );
    assert!(close(v.scale, 1.0));
    // Her waterplane: a point at the bow, her full beam from the shoulder aft.
    let plane = &v.waterplane;
    assert!(plane.cut);
    assert!(close(plane.bow, -LENGTH / 2.0) && close(plane.stern, LENGTH / 2.0));
    assert!(plane.half_breadths[0] < 0.01);
    assert!(close(plane.half_breadths[20], BEAM / 2.0));
    assert!(close(plane.beam, BEAM));
    assert_eq!(v.funnels[0].position, Vec3::new(0.0, 14.0, 2.0));
    assert_eq!(v.lamps[0].intensity, LampKind::Masthead.intensity());
    // Three materials, no textures, flat normals pointing out of her sides.
    assert_eq!(model.materials.len(), 3);
    assert!(model.textures.is_empty());
    let starboard = model
        .vertices
        .iter()
        .find(|p| close(p.position[0], BEAM / 2.0) && p.position[2] > 0.0)
        .unwrap();
    assert!(
        Vec3::from(starboard.normal).abs_diff_eq(Vec3::X, 1e-4),
        "{:?}",
        starboard.normal
    );
    assert!(model.notes.is_empty(), "{:?}", model.notes);
}

#[test]
fn a_z_up_model_in_centimetres_off_her_middle_comes_upright_in_metres() {
    let offset = Vec3::new(5000.0, 300.0, -200.0);
    // Ship space to the file's: up +Z, her bow +X (so her starboard −Y), centimetres, the origin elsewhere.
    let frame = |p: Vec3| Vec3::new(-p.z, -p.x, p.y) * 100.0 + offset;
    let glb = Glb::new(
        &Shape::ship(true),
        frame,
        vec![paint("hull", [0.4; 3], 0.6); 3],
        false,
    );
    let inspection = inspect("zup.glb", &glb.bytes(), &mut no_files).unwrap();
    assert_eq!(
        inspection.guess.up,
        Axis::Z,
        "{:?}",
        inspection.guess.reasons
    );
    assert_eq!(
        inspection.guess.forward,
        Axis::X,
        "{:?}",
        inspection.guess.reasons
    );
    assert_eq!(inspection.guessed_scale().0, 0.01);
    let starter = inspection.starter("zup.glb");
    assert_eq!(starter.length, Some(LENGTH));
    // Her waterline, as the file has it: z at her ship-space y 0.
    let setup = Setup {
        up: Axis::Z,
        forward: Axis::X,
        length: Some(LENGTH),
        waterline: Some(offset.z),
        ..Setup::new("zup.glb")
    };
    let model = load(&setup, &glb).unwrap();
    let v = &model.vessel;
    assert!(close(v.scale, 0.01), "{}", v.scale);
    assert!(
        close(v.draft, DRAFT) && close(v.beam, BEAM),
        "{} {}",
        v.draft,
        v.beam
    );
    // Every vertex is where the plain ship has it.
    let plain = load(
        &Setup {
            draft: Some(DRAFT),
            ..Setup::new("plain.glb")
        },
        &fixture::plain(),
    )
    .unwrap();
    let mine = Shape::ship(true);
    let truth: Vec<Vec3> = mine
        .parts
        .iter()
        .flat_map(|p| p.positions.iter().copied())
        .collect();
    for p in &model.vertices {
        let p = Vec3::from(p.position);
        assert!(
            truth.iter().any(|t| t.abs_diff_eq(p, 1e-3)),
            "{p} is no corner of hers"
        );
    }
    assert_eq!(model.vertices.len(), plain.vertices.len());
    // Her bow points −Z.
    let bow = model
        .vertices
        .iter()
        .map(|p| Vec3::from(p.position))
        .min_by(|a, b| a.z.total_cmp(&b.z))
        .unwrap();
    assert!(close(bow.z, -LENGTH / 2.0) && bow.x.abs() < 1e-3, "{bow}");
}

#[test]
fn the_inspection_finds_a_gltf_ships_axes() {
    let inspection = inspect("plain.glb", &fixture::plain().bytes(), &mut no_files).unwrap();
    assert_eq!(
        inspection.guess.up,
        Axis::Y,
        "{:?}",
        inspection.guess.reasons
    );
    assert_eq!(
        inspection.guess.forward,
        Axis::NEG_Z,
        "{:?}",
        inspection.guess.reasons
    );
    assert_eq!(inspection.guessed_scale(), (1.0, "metres"));
    assert_eq!(inspection.materials.len(), 3);
    let report = inspection.report(2048);
    assert!(report.contains("up +y, forward -z"), "{report}");
    let starter: Setup = Setup::from_json(inspection.starter_json("plain.glb").as_bytes()).unwrap();
    assert_eq!(starter.up, Axis::Y);
    assert_eq!(starter.draft, Some(4.0));
}

#[test]
fn a_model_with_no_normals_uvs_or_textures_draws_flat() {
    // No materials at all: glTF's default.
    let mut glb = Glb::new(&Shape::ship(false), |p| p, Vec::new(), false);
    for primitive in glb.json["meshes"][0]["primitives"].as_array_mut().unwrap() {
        primitive.as_object_mut().unwrap().remove("material");
    }
    glb.json.as_object_mut().unwrap().remove("materials");
    let model = load(&Setup::new("bare.glb"), &glb).unwrap();
    assert_eq!(model.materials.len(), 1);
    assert_eq!(model.materials[0].metallic, 1.0);
    assert!(
        model
            .vertices
            .iter()
            .all(|v| v.uv0 == [0.0; 2] && v.color == [255; 4])
    );
    // No length, scale or draft: a metre a unit and a guessed draft, both noted.
    assert_eq!(model.notes.len(), 2, "{:?}", model.notes);
}

#[test]
fn windows_blend_railings_cut_out_and_maps_keep_their_transforms() {
    let mut glb = Glb::new(&Shape::ship(true), |p| p, Vec::new(), true);
    let rail = glb.texture(4, 4, |x, _| {
        if x % 2 == 0 {
            [200, 200, 200, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    let hull = glb.texture(64, 32, |x, y| [x as u8 * 4, y as u8 * 8, 90, 255]);
    let normal = glb.texture(8, 8, |_, _| [128, 128, 255, 255]);
    glb.json["materials"] = json!([
        {
            "name": "hull",
            "pbrMetallicRoughness": {"baseColorTexture": {"index": hull, "extensions": {"KHR_texture_transform": {"scale": [4.0, 2.0], "offset": [0.5, 0.0], "texCoord": 1}}}, "metallicFactor": 0.0},
            "normalTexture": {"index": normal, "scale": 0.5},
            "emissiveFactor": [1.0, 0.8, 0.5],
            "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": 3.0}},
        },
        {"name": "bridge windows", "alphaMode": "BLEND", "doubleSided": true,
         "pbrMetallicRoughness": {"baseColorFactor": [0.1, 0.12, 0.15, 0.3], "metallicFactor": 0.0, "roughnessFactor": 0.05}},
        {"name": "railing", "alphaMode": "MASK", "alphaCutoff": 0.4, "doubleSided": true,
         "pbrMetallicRoughness": {"baseColorTexture": {"index": rail}, "metallicFactor": 0.0},
         "extensions": {"KHR_materials_transmission": {"transmissionFactor": 1.0}}},
    ]);
    glb.json["extensionsUsed"] = json!([
        "KHR_texture_transform",
        "KHR_materials_emissive_strength",
        "KHR_materials_transmission"
    ]);
    glb.json["extensionsRequired"] = json!(["KHR_texture_transform"]);
    let setup = Setup {
        texture_size: 16,
        ..Setup::new("windows.glb")
    };
    let model = load(&setup, &glb).unwrap();
    let [hull, windows, railing] = [0, 1, 2].map(|i| &model.materials[i]);
    let base = hull.maps[Slot::BaseColor as usize].unwrap();
    assert_eq!(base.set, 1);
    assert_eq!(base.transform, [[4.0, 0.0, 0.5], [0.0, 2.0, 0.0]]);
    assert_eq!(hull.emissive, [3.0, 2.4, 1.5]);
    assert_eq!(hull.normal_scale, 0.5);
    assert_eq!(windows.alpha, Alpha::Blend);
    assert!(windows.double_sided);
    // A cut-out stays cut out, transmission or not.
    assert_eq!(railing.alpha, Alpha::Mask(0.4));
    // Three images, the hull's halved to fit 16 and every chain down to a texel.
    assert_eq!(model.textures.len(), 3);
    let hull_map = &model.textures[base.texture];
    assert_eq!((hull_map.width(), hull_map.height()), (16, 8));
    assert_eq!(hull_map.source_size, [64, 32]);
    assert!(hull_map.srgb);
    assert_eq!(
        hull_map.levels.last().map(|l| (l.width, l.height)),
        Some((1, 1))
    );
    let normal_map = &model.textures[hull.maps[Slot::Normal as usize].unwrap().texture];
    assert!(!normal_map.srgb);
    // Glass by transmission blends, its reflections kept.
    glb.json["materials"][1] = json!({"name": "glass", "extensions": {"KHR_materials_transmission": {"transmissionFactor": 0.9}}});
    let glass = load(&setup, &glb).unwrap();
    assert_eq!(glass.materials[1].alpha, Alpha::Blend);
    assert!(glass.materials[1].transmission > 0.8);
}

fn refused(glb: &Glb) -> String {
    format!("{:#}", load(&Setup::new("bad.glb"), glb).unwrap_err())
}

#[test]
fn what_cannot_be_drawn_is_refused_with_its_reason() {
    let mut draco = fixture::plain();
    draco.json["extensionsUsed"] = json!(["KHR_draco_mesh_compression"]);
    draco.json["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
    let reason = refused(&draco);
    assert!(reason.contains("Draco"), "{reason}");

    let mut unknown = fixture::plain();
    unknown.json["extensionsRequired"] = json!(["EXT_made_up"]);
    assert!(refused(&unknown).contains("EXT_made_up"));

    let mut webp = fixture::plain();
    webp.json["textures"] = json!([{"extensions": {"EXT_texture_webp": {"source": 0}}}]);
    webp.json["images"] = json!([{"uri": "hull.webp"}]);
    let reason = refused(&webp);
    assert!(reason.contains("EXT_texture_webp"), "{reason}");

    let mut quantized = fixture::plain();
    quantized.json["accessors"][0]["componentType"] = json!(5123);
    assert!(refused(&quantized).contains("float vec3"));

    let mut past = fixture::plain();
    past.json["accessors"][0]["count"] = json!(100_000);
    let reason = refused(&past);
    assert!(reason.contains("outside"), "{reason}");

    let mut indices = fixture::plain();
    let index_view = indices.json["accessors"][1]["bufferView"].as_u64().unwrap() as usize;
    let offset = indices.json["bufferViews"][index_view]["byteOffset"]
        .as_u64()
        .unwrap() as usize;
    indices.bin[offset..offset + 4].copy_from_slice(&9999u32.to_le_bytes());
    assert!(refused(&indices).contains("index past"));

    let mut old = fixture::plain();
    old.json["asset"]["version"] = json!("1.0");
    assert!(refused(&old).contains("glTF 2.0"));

    let mut outside = fixture::plain();
    outside.json["buffers"] = json!([{"uri": "ship.bin", "byteLength": 10}]);
    assert!(
        format!(
            "{:#}",
            VesselModel::load(
                &Setup::new("x.gltf"),
                "x",
                &serde_json::to_vec(&outside.json).unwrap(),
                &mut no_files
            )
            .unwrap_err()
        )
        .contains("outside file")
    );

    for junk in [&b""[..], b"glTF", b"{not json", &[0u8; 64]] {
        assert!(VesselModel::load(&Setup::new("junk.glb"), "junk", junk, &mut no_files).is_err());
    }
}

#[test]
fn a_gltf_file_reads_its_buffers_and_images_beside_it() {
    // The plain ship as a .gltf with its buffer in a file of its own.
    let glb = fixture::plain();
    let mut json = glb.json.clone();
    json["buffers"] = json!([{"uri": "plain%20ship.bin", "byteLength": glb.bin.len()}]);
    let bin = glb.bin.clone();
    let mut asked = Vec::new();
    let mut resolver = |uri: &str| -> anyhow::Result<Vec<u8>> {
        asked.push(uri.to_owned());
        Ok(bin.clone())
    };
    let model = VesselModel::load(
        &Setup {
            draft: Some(DRAFT),
            ..Setup::new("plain.gltf")
        },
        "plain",
        &serde_json::to_vec(&json).unwrap(),
        &mut resolver,
    )
    .unwrap();
    assert_eq!(asked, vec!["plain ship.bin".to_owned()]);
    assert!(close(model.vessel.length, LENGTH));
}

/// `glb` refused with `reason`, by this crate's checks (not the panic backstop's).
fn refused_for(glb: &Glb, reason: &str) {
    let got = refused(glb);
    assert!(
        got.contains(reason) && !got.contains("reader failed"),
        "{reason:?} not in {got:?}"
    );
    let inspected = inspect("bad.glb", &glb.bytes(), &mut no_files).map(|_| ());
    assert!(inspected.is_err(), "inspected: {reason}");
}

#[test]
fn malformed_accessors_are_refused_before_they_are_read() {
    // A POSITION past the accessors: `gltf`'s validation indexes them by it.
    let mut glb = fixture::plain();
    glb.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(999);
    refused_for(&glb, "POSITION is accessor 999");

    // UVs of unsigned ints and float indices: `gltf`'s readers reach `unreachable!()`.
    let mut glb = fixture::plain();
    let mut uvs = glb.json["accessors"][0].clone();
    uvs["componentType"] = json!(5125);
    uvs["type"] = json!("VEC2");
    uvs.as_object_mut()
        .unwrap()
        .retain(|k, _| k != "min" && k != "max");
    glb.json["accessors"].as_array_mut().unwrap().push(uvs);
    glb.json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(6);
    refused_for(&glb, "UVs (accessor 6) are U32");
    let mut glb = fixture::plain();
    glb.json["accessors"][1]["componentType"] = json!(5126);
    refused_for(&glb, "indices (accessor 1) are F32");

    // A stride shorter than a position: the readers assert.
    let mut glb = fixture::plain();
    glb.json["bufferViews"][0]["byteStride"] = json!(4);
    refused_for(&glb, "stride of 4 bytes");
}

#[test]
fn malformed_sparse_accessors_are_refused() {
    let sparse = |count: u64, offset: u64| -> Glb {
        let mut glb = fixture::plain();
        let view = glb.json["accessors"][1]["bufferView"].clone();
        glb.json["accessors"][1]["sparse"] = json!({
            "count": count,
            "indices": {"bufferView": view, "byteOffset": offset, "componentType": 5125},
            "values": {"bufferView": view},
        });
        glb
    };
    // None: the reader takes one from it.
    refused_for(&sparse(0, 0), "sparse count is 0");
    // An offset whose sum overflows.
    refused_for(&sparse(2, u64::MAX - 4), "sparse indices");
}

#[test]
fn counts_are_bounded_before_they_are_read() {
    // A sparse index accessor of four bytes naming four billion indices (16 GB once read).
    let mut glb = fixture::plain();
    let view = glb.view(&0u32.to_le_bytes(), None);
    let accessors = glb.json["accessors"].as_array_mut().unwrap();
    accessors.push(json!({
        "componentType": 5125, "count": 4_000_000_000u64, "type": "SCALAR",
        "sparse": {"count": 1, "indices": {"bufferView": view, "componentType": 5125}, "values": {"bufferView": view}},
    }));
    let index = accessors.len() - 1;
    glb.json["meshes"][0]["primitives"][0]["indices"] = json!(index);
    refused_for(&glb, "triangles: decimate");
}

#[test]
fn only_buffer_0_is_the_binary_chunk() {
    // A thousand buffers with no URI, each of which would have copied the chunk.
    let mut glb = fixture::plain();
    let buffers = glb.json["buffers"].as_array_mut().unwrap();
    buffers.extend((0..1000).map(|_| json!({"byteLength": 8})));
    refused_for(&glb, "only buffer 0");
}

#[test]
fn a_scene_is_walked_as_a_forest() {
    // A chain of 48 nodes, each listing the next twice: 2⁴⁸ visits walked as a tree.
    let mut glb = fixture::plain();
    let mut nodes: Vec<serde_json::Value> = (1..48).map(|k| json!({"children": [k, k]})).collect();
    nodes.push(json!({"mesh": 0}));
    glb.json["nodes"] = json!(nodes);
    refused_for(&glb, "node 47 is reached twice");
    // A cycle.
    glb.json["nodes"] = json!([{"mesh": 0, "children": [1]}, {"children": [0]}]);
    refused_for(&glb, "reached twice");
}

#[test]
fn each_map_keeps_its_sampler_and_only_coverage_weighs_mips() {
    let mut glb = Glb::new(&Shape::ship(true), |p| p, Vec::new(), true);
    let rail = glb.texture(4, 4, |x, _| {
        [200, 200, 200, if x % 2 == 0 { 255 } else { 0 }]
    });
    let glow = glb.texture(4, 4, |_, _| [255, 200, 100, 255]);
    // The railing's map sharp-edged and clamped across; the glow's glTF's default.
    glb.json["samplers"] = json!([{}, {"magFilter": 9728, "minFilter": 9728, "wrapS": 33071}]);
    glb.json["textures"][rail]["sampler"] = json!(1);
    glb.json["materials"] = json!([
        {"name": "hull", "pbrMetallicRoughness": {"baseColorTexture": {"index": rail}}, "emissiveTexture": {"index": glow}},
        {"name": "railing", "alphaMode": "MASK", "pbrMetallicRoughness": {"baseColorTexture": {"index": rail}}},
        paint("mast", [0.1; 3], 0.5),
    ]);
    let model = load(&Setup::new("maps.glb"), &glb).unwrap();
    let [hull, railing] = [0, 1].map(|i| &model.materials[i]);
    let base = hull.maps[Slot::BaseColor as usize].unwrap();
    let sharp = Sampling {
        wrap: [Wrap::Clamp, Wrap::Repeat],
        mag: Filter::Nearest,
        min: Filter::Nearest,
        mip: None,
    };
    assert_eq!(base.sampling, sharp);
    assert_eq!(
        hull.maps[Slot::Emissive as usize].unwrap().sampling,
        Sampling::default()
    );
    let descriptor = sharp.descriptor();
    assert_eq!(
        (descriptor.anisotropy_clamp, descriptor.lod_max_clamp),
        (1, 0.0)
    );
    assert_eq!(Sampling::default().descriptor().anisotropy_clamp, 8);
    // The one image twice: the opaque hull's colour box-filtered, the railing's weighted by its coverage.
    let cut = railing.maps[Slot::BaseColor as usize].unwrap().texture;
    assert_ne!(base.texture, cut);
    assert!(!model.textures[base.texture].coverage && model.textures[cut].coverage);

    // A map on a third UV set is refused by name.
    glb.json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(2);
    refused_for(&glb, "TEXCOORD_2");
}
