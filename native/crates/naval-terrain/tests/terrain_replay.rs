//! The terrain preparation, level of detail and review cameras over the test land (`support::land`), and every map's
//! battle environments, against the values recorded in `tests/fixtures/terrain.json` (hashes of the prepared buffers,
//! and the numbers themselves). The file began as pins read from the removed web game's TypeScript, each battle map's
//! land among them; it is recorded from this crate, so it guards against unintended change. (The land maps were
//! removed on 2026-10-27: the test land stands in for their land.)
//!
//! A deliberate change (to the land's preparation, the test land or an environment) re-records it:
//! `UPDATE_FIXTURES=1 cargo test -p naval-terrain --test all -- terrain_replay::`; review the diff and commit it.

use crate::support;
use native_offscreen::fixture::check_or_record;
use naval_conditions::BattleConditions;
use naval_terrain::field::{self, Grid};
use naval_terrain::heightfield::Heightfield;
use naval_terrain::lod::{
    LodSettings, LodView, PATCH_CAPACITY, PATCH_FLOATS, TerrainChart, TerrainLod,
};
use naval_terrain::{noise, review};
use serde_json::{Map, Value, json};
use std::path::Path;

/// The sun the land's light is baked under.
const SUN_DIRECTION: [f64; 3] = [
    0.350_596_521_561_703_34,
    0.500_852_173_659_576_2,
    -0.791_346_434_382_130_5,
];

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    u64::from(h)
}

/// A land: its field, its prepared buffers, the level of detail's choices for three views and the review page's camera
/// searches.
fn land(field: &Heightfield) -> Map<String, Value> {
    let quanta: Vec<u8> = field.quanta.iter().flat_map(|q| q.to_le_bytes()).collect();
    let heights: Vec<f64> = [
        [0.0, 0.0],
        [1234.5, -2345.25],
        [-8000.0, 4000.0],
        [5000.0, 5000.0],
    ]
    .iter()
    .map(|&[x, z]| field.height(x, z))
    .collect();

    let prepared = field::PreparedField::new(field);
    let gradients: Vec<u8> = prepared
        .gradients
        .iter()
        .flat_map(|g| g.to_le_bytes())
        .collect();
    let mut sun = Vec::new();
    prepared.sunlight(SUN_DIRECTION, &mut sun);
    let bounds = &prepared.bounds;
    let top = bounds.levels.last().unwrap();

    let [min_x, min_z, max_x, max_z] = field.bounds();
    let mut lod = TerrainLod::new(
        TerrainChart {
            origin_x: min_x,
            origin_z: min_z,
            width: max_x - min_x,
            depth: max_z - min_z,
            bounds: prepared.bounds.clone(),
            cell: f64::from(field.cell),
        },
        LodSettings {
            spacing: 20.0,
            target_pixels: 5.0,
            deep_water_m: naval_terrain::lod::DEEP_WATER_M as f64,
        },
    )
    .unwrap();
    let views = [
        LodView {
            position: [2000.0, 40.0, -6000.0],
            planes: vec![],
            fov_y: 0.9,
            height_px: 900.0,
            submerged: false,
            curvature: 0.0,
        },
        LodView {
            position: [0.0, 30.0, 0.0],
            planes: vec![
                [0.2, 0.05, -0.97, 800.0],
                [-0.6, 0.1, -0.8, 400.0],
                [0.0, 0.99, 0.1, 50.0],
            ],
            fov_y: 0.9,
            height_px: 900.0,
            submerged: false,
            curvature: 0.0,
        },
        LodView {
            position: [-5000.0, 900.0, 3000.0],
            planes: vec![],
            fov_y: 0.2,
            height_px: 1080.0,
            submerged: false,
            curvature: 0.0,
        },
    ];
    let mut buffer = vec![0f32; PATCH_CAPACITY * PATCH_FLOATS];
    let selections: Vec<Value> = views
        .iter()
        .map(|view| {
            let count = lod.select(view, &mut buffer);
            let bytes: Vec<u8> = buffer[..count * PATCH_FLOATS]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            json!({
                "count": count,
                "nearLand": lod.near_land,
                "range0": lod.ranges[0],
                "treeReach": if lod.tree_reach.is_finite() { lod.tree_reach } else { -1.0 },
                "hash": fnv(&bytes),
            })
        })
        .collect();

    let coast = review::main_coast(field);
    let steep = review::steepest_coast(field);
    let value = json!({
        "columns": field.columns,
        "rows": field.rows,
        "cell": field.cell,
        "originX": field.origin_x,
        "originZ": field.origin_z,
        "step": field.step,
        "quanta": fnv(&quanta),
        "heightsAt": heights,
        "gradients": fnv(&gradients),
        "coarse": [prepared.coarse.columns, prepared.coarse.rows],
        "attributes": fnv(&prepared.attributes),
        "sun": fnv(&sun),
        "bounds": {"levels": bounds.levels.len(), "top": [top.min[0], top.max[0]]},
        "selections": selections,
        "mainCoast": {"direction": coast.direction, "hit": coast.hit, "angle": coast.angle},
        "steepestCoast": {"x": steep.x, "z": steep.z, "dx": steep.dx, "dz": steep.dz, "height": steep.height},
    });
    match value {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

/// A map's spawn distance and its battle environment under four sets of conditions.
fn environments(map: &naval_conditions::OceanMap) -> Map<String, Value> {
    let custom = BattleConditions {
        time_hours: Some(7.5),
        cloud_cover: Some(55.0),
        wind_speed: Some(14.0),
        visibility_km: Some(12.0),
        ..BattleConditions::default()
    };
    let cases = [
        ("map", "map", "map", BattleConditions::default()),
        (
            "duskOvercast",
            "dusk",
            "overcast",
            BattleConditions::default(),
        ),
        (
            "nightStorm",
            "night",
            "storm-clouds",
            BattleConditions::default(),
        ),
        ("custom", "map", "clear", custom),
    ];
    let mut environments = Map::new();
    for (name, time, weather, conditions) in &cases {
        let e = naval_conditions::battle_environment(map, time, weather, conditions).unwrap();
        let sky = &e.sky;
        environments.insert(
            (*name).into(),
            json!({
                "sky": {
                    "elevation": sky.elevation,
                    "azimuth": sky.azimuth,
                    "intensity": sky.intensity,
                    "coverage": sky.coverage,
                    "altitude": sky.altitude,
                    "thickness": sky.thickness,
                    "turbidity": sky.turbidity,
                    "rayleigh": sky.rayleigh,
                    "mie": sky.mie,
                    "mieG": sky.mie_g,
                    "multiple": sky.multiple,
                    "ambient": sky.ambient,
                },
                "fog": {
                    "color": e.fog.color,
                    "start": e.fog.start,
                    "end": e.fog.end,
                    "skyBlend": e.fog.sky_blend,
                },
                "cloudWind": e.cloud_wind,
                "waves": {
                    "windSpeed": e.waves.wind_speed,
                    "significantHeightM": e.waves.significant_height,
                    "peakWavelength": e.waves.peak_wavelength,
                    "choppiness": e.waves.choppiness,
                },
                "waterLightScale": e.water_light_scale,
                "cloudAmbient": e.cloud_ambient,
                "cloudShadow": e.cloud_shadow,
                "horizonCoverage": e.horizon_coverage,
            }),
        );
    }
    let mut out = Map::new();
    out.insert("spawnDistance".into(), json!(map.spawn_distance()));
    out.insert("environments".into(), Value::Object(environments));
    out
}

#[test]
fn the_land_prepares_and_every_map_resolves_its_environments_as_recorded() {
    let mut maps = Map::new();
    for map in naval_conditions::maps() {
        maps.insert(map.id.clone(), Value::Object(environments(map)));
    }
    let recorded = json!({
        "source": "naval-terrain; re-record with UPDATE_FIXTURES=1 cargo test -p naval-terrain --test all -- terrain_replay::",
        "noise": fnv(&noise::terrain_noise()),
        "land": land(&support::land()),
        "maps": maps,
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terrain.json");
    check_or_record(&path, &recorded, 1e-12);
}

#[test]
fn the_north_atlantic_selection_is_the_sky_lanes_battle_sky() {
    let scene = naval_conditions::MapSelection::battle("north-atlantic")
        .resolve()
        .unwrap();
    let reference = naval_sky::SkyScene::north_atlantic(48.0, 235.0);
    assert_eq!(scene.sky, reference);
    assert!(scene.terrain.is_none());
    assert_eq!(scene.offset, [0.0, -2500.0]);
    let port = naval_conditions::MapSelection::port().resolve().unwrap();
    assert_eq!(port.sky, naval_sky::state::DEFAULT_SKY_SCENE);
    // The port's fair-day haze (22 km visibility), not the web's 5.6 km showroom fog.
    assert_eq!(port.fog.end, naval_ocean::calibration::PORT_FOG.1);
    // The chart's bearing turns the authored sun: 235° true on a chart whose top bears 75°.
    let mut turned = scene.map.clone();
    turned.bearing = 75.0;
    let environment =
        naval_conditions::battle_environment(&turned, "map", "map", &BattleConditions::default())
            .unwrap();
    assert_eq!(environment.sky.azimuth, (235.0 + 75.0) % 360.0);
    let _ = Grid::of;
}
