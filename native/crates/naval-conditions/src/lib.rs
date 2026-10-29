//! The scene's conditions: the map (the North Atlantic's open sea: the land maps were removed on 2026-10-27), the
//! time of day, the weather, the sea and the fog a battle, a theatre scene or the port is drawn in, and how the game
//! turns them into the sky, the sea and the fog: `src/weather/catalog.ts`, `conditions.ts` (`battleEnvironment`),
//! `clouds.ts` (`resolveClouds`), `precipitation.ts` (`battlePrecipitation`) and the scene half of the web game's
//! `VisualEnvironment.ts` (`applySea`, `applyLighting`, `applyFog`). The land's look a scene carries (its style,
//! palette and climate) is [`style`]. See `native/docs/terrain/conditions.md`, "The map selection API".
//!
//! The map data is the game's own JSON (`assets/maps/environments.v1.json`, `battle-conditions.v1.json`), embedded at
//! build time, so the crate reads no file at run time and follows every edit to the maps. [`MapSelection`] is the map
//! selection API the client uses: a map, a time of day, a weather and optional conditions resolve to a [`MapScene`]
//! (a `naval_sky::SkyScene`, `naval_ocean::OceanParameters`, the fog, the mesh ambient, the land's style and where the
//! battle places its chart).
//!
//! Portable (checked for `wasm32-unknown-unknown`). It was `naval_terrain::catalog` until 2026-10-28.

pub mod style;

pub use style::{Climate, LandStyle, SurfaceStyle};

use std::sync::OnceLock;

use naval_ocean::calibration::{self, MapWater};
use naval_ocean::{FogParameters, OceanParameters};
use naval_sky::model::AuthoredAtmosphere;
use naval_sky::state::SkyAngles;
use naval_sky::{CloudGenus, CloudScene, HighCloudGenus, HighClouds, SkyScene, WeatherScene};
use serde::Deserialize;

const ENVIRONMENTS_JSON: &str = include_str!("../../../../assets/maps/environments.v1.json");
const CONDITIONS_JSON: &str = include_str!("../../../../assets/maps/battle-conditions.v1.json");

/// `OceanMap.water`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapWaterColors {
    pub water_color: String,
    pub transmission_color: String,
    pub absorption_color: String,
    pub amplitude_scale: f64,
    pub wind_scale: f64,
    pub wavelength_scale: f64,
    /// Degrees in the chart frame, as the simulation reads it.
    pub wind_direction: f64,
    pub foam: f64,
}

/// `OceanMap.sky`: `azimuth` is a true direction in the sky's convention; the battle turns it into the chart frame.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSky {
    pub elevation: f64,
    pub azimuth: f64,
    pub intensity: f64,
    pub coverage: f64,
    pub altitude: f64,
    pub thickness: f64,
    pub turbidity: f64,
    pub rayleigh: f64,
    pub mie: f64,
    pub mie_g: f64,
    pub multiple: f64,
    pub ambient: f64,
}

/// `OceanMap.fog`: colour as sRGB hex, distances in metres.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapFog {
    pub color: String,
    pub start: f64,
    pub end: f64,
    pub sky_blend: f64,
}

/// `OceanMap.land`: the style and its palette (sRGB hex), and the id of a baked heightfield: `None`, open sea (the land
/// maps' heightfields were removed on 2026-10-27; a theatre hands its windows' land to the layer itself).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapLand {
    pub style: LandStyle,
    pub shore: String,
    pub low: String,
    pub high: String,
    pub terrain: Option<String>,
}

/// A battle map (`OceanMap`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OceanMap {
    pub id: String,
    pub name: String,
    pub region: String,
    /// True bearing of the chart's top, degrees clockwise from north.
    pub bearing: f64,
    pub battle: Option<String>,
    pub description: String,
    pub water: MapWaterColors,
    pub sky: MapSky,
    pub fog: MapFog,
    pub land: MapLand,
    /// Metres between the default spawn lines (`spawnDistanceM`); `None` is [`DEFAULT_SPAWN_DISTANCE_M`].
    pub spawn_distance_m: Option<f64>,
}

impl OceanMap {
    /// The map's water in the ocean crate's table (`naval_ocean::calibration::MAP_WATERS`).
    pub fn ocean_water(&self) -> &'static MapWater {
        calibration::map_water(&self.id).unwrap_or(&calibration::NORTH_ATLANTIC)
    }

    /// The spawn distance staged battles open at on this map (`mapSpawnDistance`).
    pub fn spawn_distance(&self) -> f64 {
        self.spawn_distance_m.unwrap_or(DEFAULT_SPAWN_DISTANCE_M)
    }

    /// Where a staged battle at `spawn_distance` places the chart in its world (`customTerrainOffset`): world =
    /// chart + offset.
    pub fn custom_offset(spawn_distance: f64) -> [f64; 2] {
        [0.0, -spawn_distance / 2.0]
    }

    /// The land's palette and style as the terrain shades it (`BattleLandscape.surfaceStyle`).
    pub fn surface_style(&self) -> SurfaceStyle {
        let style = self.land.style;
        // The styles with seasons are northern ones (as Norway's and the Channel's were): the equator lies true south,
        // (sin B, cos B) on a chart whose top has true bearing B. The tropical styles take no aspect.
        let seasonal = matches!(style, LandStyle::Snow | LandStyle::Chalk);
        let bearing = self.bearing.to_radians();
        SurfaceStyle {
            style,
            shore: srgb_hex(&self.land.shore),
            low: srgb_hex(&self.land.low),
            high: srgb_hex(&self.land.high),
            equator: if seasonal {
                [bearing.sin() as f32, bearing.cos() as f32]
            } else {
                [0.0, 0.0]
            },
            climate: Climate::AUTHORED,
        }
    }
}

/// The spawn distance on a map that sets none (`DEFAULT_SPAWN_DISTANCE_M`).
pub const DEFAULT_SPAWN_DISTANCE_M: f64 = 5000.0;
/// The map every battle defaults to.
pub const DEFAULT_MAP: &str = "north-atlantic";

#[derive(Deserialize)]
struct EnvironmentsFile {
    maps: Vec<OceanMap>,
}

/// Every battle map, in the game's order.
pub fn maps() -> &'static [OceanMap] {
    static MAPS: OnceLock<Vec<OceanMap>> = OnceLock::new();
    MAPS.get_or_init(|| {
        serde_json::from_str::<EnvironmentsFile>(ENVIRONMENTS_JSON)
            .expect("assets/maps/environments.v1.json parses")
            .maps
    })
}

/// A map by id (`oceanMap`).
pub fn map(id: &str) -> Option<&'static OceanMap> {
    maps().iter().find(|map| map.id == id)
}

/// Sky fields a preset replaces (`Partial<OceanMap['sky']>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SkyOverride {
    pub elevation: Option<f64>,
    pub azimuth: Option<f64>,
    pub intensity: Option<f64>,
    pub coverage: Option<f64>,
    pub altitude: Option<f64>,
    pub thickness: Option<f64>,
    pub turbidity: Option<f64>,
    pub rayleigh: Option<f64>,
    pub mie: Option<f64>,
    pub mie_g: Option<f64>,
    pub multiple: Option<f64>,
    pub ambient: Option<f64>,
}

impl SkyOverride {
    fn apply(&self, sky: &mut MapSky) {
        let set = |field: &mut f64, value: Option<f64>| {
            if let Some(value) = value {
                *field = value;
            }
        };
        set(&mut sky.elevation, self.elevation);
        set(&mut sky.azimuth, self.azimuth);
        set(&mut sky.intensity, self.intensity);
        set(&mut sky.coverage, self.coverage);
        set(&mut sky.altitude, self.altitude);
        set(&mut sky.thickness, self.thickness);
        set(&mut sky.turbidity, self.turbidity);
        set(&mut sky.rayleigh, self.rayleigh);
        set(&mut sky.mie, self.mie);
        set(&mut sky.mie_g, self.mie_g);
        set(&mut sky.multiple, self.multiple);
        set(&mut sky.ambient, self.ambient);
    }
}

/// Fog fields a weather replaces.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FogOverride {
    pub color: Option<String>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub sky_blend: Option<f64>,
}

/// A time of day (`TIME_OF_DAY_PRESETS`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimePreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sky: SkyOverride,
    pub light_scale: f64,
    pub fog_color: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetWaves {
    pub wind_speed: f64,
}

/// A weather (`WEATHER_PRESETS`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sky: SkyOverride,
    pub fog: FogOverride,
    pub sun_scale: f64,
    pub ambient_scale: f64,
    pub cloud_wind: f64,
    pub waves: PresetWaves,
}

#[derive(Deserialize)]
struct ConditionsFile {
    times: Vec<TimePreset>,
    weather: Vec<WeatherPreset>,
}

fn conditions_file() -> &'static ConditionsFile {
    static FILE: OnceLock<ConditionsFile> = OnceLock::new();
    FILE.get_or_init(|| {
        serde_json::from_str(CONDITIONS_JSON).expect("assets/maps/battle-conditions.v1.json parses")
    })
}

/// The times of day a battle may choose (`map`, `dawn`, `morning`, `noon`, `dusk`, `night`).
pub fn time_presets() -> &'static [TimePreset] {
    &conditions_file().times
}

/// The weathers a battle may choose (`map`, `clear`, `partly-cloudy`, `overcast`, `fog`, `storm-clouds`).
pub fn weather_presets() -> &'static [WeatherPreset] {
    &conditions_file().weather
}

/// The layer's cloud genus a battle chooses, or `Auto` (the cover decides): `CloudChoice`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudChoice {
    Auto,
    Genus(CloudGenus),
}

/// The high sheet's genus, `None` for a clear sky above the layer, or `Auto` (`HighCloudChoice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighCloudChoice {
    Auto,
    None,
    Genus(HighCloudGenus),
}

/// What a battle, or the developer console, asks of the weather beyond its presets (`BattleConditions` with the
/// console's `CloudConditions`). Every field left `None` keeps the preset's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BattleConditions {
    pub time_hours: Option<f64>,
    /// The sun where it stands, in place of the hour's (a theatre's sun over its scene, `naval_sim::theatre::sun`).
    pub sun: Option<Sun>,
    /// The moon where it stands and its phase (a theatre's moon over its scene, `naval_sim::theatre::moon`). None is
    /// a full moon opposite the sun.
    pub moon: Option<Moon>,
    /// Percent.
    pub cloud_cover: Option<f64>,
    /// m/s.
    pub wind_speed: Option<f64>,
    pub visibility_km: Option<f64>,
    pub cloud_type: Option<CloudChoice>,
    /// Metres.
    pub cloud_base: Option<f64>,
    pub cloud_depth: Option<f64>,
    pub high_cloud: Option<HighCloudChoice>,
    /// Percent.
    pub high_cloud_cover: Option<f64>,
    pub high_cloud_base: Option<f64>,
    /// Degrees the wind blows toward in the chart frame (the map's `water.windDirection`, which it replaces): the
    /// clouds drift with it. A theatre's scenes give their scenario's wind.
    pub wind_direction: Option<f64>,
}

/// A sun set outright: degrees above the horizon, its azimuth in the sky's angles (0 toward +Z, 90 toward +X) before
/// the map's bearing turns it, and whether it is the morning's (the dawn's twilight haze, else the dusk's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    pub elevation: f64,
    pub azimuth: f64,
    pub morning: bool,
}

/// A moon set outright: degrees above the horizon, its azimuth in the sky's angles before the map's bearing turns it,
/// and its phase (0 new, 0.5 full).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moon {
    pub elevation: f64,
    pub azimuth: f64,
    pub phase: f64,
}

/// The sun at `hour` of `battleEnvironment`'s day: up at 6, 70° at noon, down at 18, round the sky at 15° an hour.
pub fn sun_at_hour(hour: f64) -> Sun {
    Sun {
        elevation: 70.0 * ((hour - 6.0) * std::f64::consts::PI / 12.0).sin(),
        azimuth: (hour * 15.0).rem_euclid(360.0),
        morning: hour.rem_euclid(24.0) <= 12.0,
    }
}

/// The hour of [`sun_at_hour`]'s day whose sun stands at `elevation`, on the side of the sky `azimuth` lies on (east:
/// the morning, or before midnight below the horizon), and the turn (degrees) from that hour's azimuth to `azimuth`: a
/// time preset's or a map's own sun lies off the day's arc, and keeps that offset as it moves on.
pub fn hour_of_sun(elevation: f64, azimuth: f64) -> (f64, f64) {
    use std::f64::consts::PI;
    let rise = (elevation / 70.0).clamp(-1.0, 1.0).asin() * 12.0 / PI;
    let east = azimuth.rem_euclid(360.0) < 180.0;
    let hour = match (elevation >= 0.0, east) {
        (true, true) => 6.0 + rise,
        (true, false) => 18.0 - rise,
        // Below the horizon: after sunset in the west, before sunrise in the east.
        (false, false) => 18.0 - rise,
        (false, true) => 30.0 + rise,
    }
    .rem_euclid(24.0);
    let offset = (azimuth - sun_at_hour(hour).azimuth + 540.0).rem_euclid(360.0) - 180.0;
    (hour, offset)
}

/// The calibrated wind sea (`windSea`), as `battleEnvironment` returns it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindSea {
    pub wind_speed: f64,
    pub significant_height: f64,
    pub peak_wavelength: f64,
    pub choppiness: f64,
}

/// What `battleEnvironment` resolves: the sky block turned into the chart frame, the fog, the sea and the light scales.
#[derive(Clone, Debug, PartialEq)]
pub struct BattleEnvironment {
    pub sky: MapSky,
    pub fog: MapFog,
    pub cloud_wind: f64,
    pub waves: WindSea,
    /// The water's own light under water (its pigment, which the underwater view and the sea's underside take as
    /// emitted), a share of the day's: [`water_light_scale`].
    pub water_light_scale: f64,
    pub cloud_ambient: f64,
    pub cloud_shadow: f64,
    pub horizon_coverage: f64,
    /// Fog banks lying on the sea, 0–1: the fog weather's (`naval_sky::CloudScene::fog`).
    pub fog_banks: f64,
    /// The wind speed the sea was drawn for (m/s), before calibration.
    pub wind: f64,
}

fn smooth(value: f64, start: f64, end: f64) -> f64 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Interpolates two sRGB hex swatches channel by channel, as `blendFog` does (JavaScript's `Math.round`).
fn blend_fog(from: &str, to: &str, weight: f64) -> String {
    let a = hex_value(from);
    let b = hex_value(to);
    let channel = |shift: u32| {
        let x =
            f64::from((a >> shift) & 255) * (1.0 - weight) + f64::from((b >> shift) & 255) * weight;
        (x + 0.5).floor() as u32
    };
    format!("#{:02x}{:02x}{:02x}", channel(16), channel(8), channel(0))
}

fn hex_value(hex: &str) -> u32 {
    u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0)
}

/// three's `Color.set(hex)`: an sRGB hex swatch in linear light.
pub fn srgb_hex(hex: &str) -> [f32; 3] {
    calibration::srgb_hex(hex_value(hex))
}

/// A time preset by id.
pub fn time_preset(id: &str) -> Option<&'static TimePreset> {
    time_presets().iter().find(|preset| preset.id == id)
}

/// A weather preset by id.
pub fn weather_preset(id: &str) -> Option<&'static WeatherPreset> {
    weather_presets().iter().find(|preset| preset.id == id)
}

/// `battleEnvironment(map, timeOfDay, weather, conditions)`: shared wind and sea conditions beside the visual sky and
/// fog. Sun azimuths come out in the map's chart frame, turned by its bearing. `None` for an unknown preset.
pub fn battle_environment(
    map: &OceanMap,
    time_of_day: &str,
    weather: &str,
    conditions: &BattleConditions,
) -> Option<BattleEnvironment> {
    let time = time_preset(time_of_day)?;
    let forecast = weather_preset(weather)?;
    let mut sky = map.sky;
    forecast.sky.apply(&mut sky);
    time.sky.apply(&mut sky);
    sky.intensity *= forecast.sun_scale;
    sky.ambient *= time.light_scale * forecast.ambient_scale;
    let mut fog = map.fog.clone();
    if let Some(color) = &forecast.fog.color {
        fog.color = color.clone();
    }
    if let Some(start) = forecast.fog.start {
        fog.start = start;
    }
    if let Some(end) = forecast.fog.end {
        fog.end = end;
    }
    if let Some(blend) = forecast.fog.sky_blend {
        fog.sky_blend = blend;
    }
    if let Some(color) = &time.fog_color {
        fog.color = color.clone();
    }
    // A chosen visibility moves the whole fog ramp, keeping the weather's proportions.
    if let Some(km) = conditions.visibility_km {
        let scale = km * 1000.0 / fog.end;
        fog.start *= scale;
        fog.end = km * 1000.0;
        fog.sky_blend *= scale;
    }
    let sun = conditions.sun.or(conditions.time_hours.map(sun_at_hour));
    if let Some(sun) = sun {
        sky.elevation = sun.elevation;
        sky.azimuth = sun.azimuth;
        let color = |id: &str| {
            time_preset(id)
                .and_then(|t| t.fog_color.clone())
                .unwrap_or_default()
        };
        let twilight = if sun.morning {
            color("dawn")
        } else {
            color("dusk")
        };
        fog.color = if sky.elevation < 5.0 {
            blend_fog(&color("night"), &twilight, smooth(sky.elevation, -6.0, 5.0))
        } else {
            blend_fog(&twilight, &fog.color, smooth(sky.elevation, 5.0, 18.0))
        };
    }
    // The chart's top bears `map.bearing`: a true direction lies that much further round in the chart frame.
    sky.azimuth = ((sky.azimuth + map.bearing) % 360.0 + 360.0) % 360.0;
    let daylight = smooth(sky.elevation, -6.0, 18.0);
    if sun.is_some() || time_of_day != "map" {
        sky.ambient = map.sky.ambient * forecast.ambient_scale * (0.5 + 0.5 * daylight);
    }
    if let Some(cover) = conditions.cloud_cover {
        sky.coverage = cover / 100.0;
    }
    let wind = conditions
        .wind_speed
        .unwrap_or(forecast.waves.wind_speed * map.water.wind_scale);
    let (wind_speed, significant_height, peak_wavelength, choppiness) =
        calibration::wind_sea(map.ocean_water(), wind);
    Some(BattleEnvironment {
        sky,
        fog,
        cloud_wind: conditions
            .wind_speed
            .map_or(forecast.cloud_wind, |w| w * 4.0 / 3.0),
        waves: WindSea {
            wind_speed,
            significant_height,
            peak_wavelength,
            choppiness,
        },
        water_light_scale: water_light_scale(sky.elevation),
        cloud_ambient: 1.1 * forecast.ambient_scale,
        cloud_shadow: if weather == "storm-clouds" { 0.55 } else { 0.2 },
        horizon_coverage: match conditions.cloud_cover {
            Some(cover) => 0.06 * cover / 100.0,
            None if weather == "clear" => 0.0,
            None => 0.06,
        },
        fog_banks: if weather == "fog" { 1.0 } else { 0.0 },
        wind,
    })
}

/// The water's own light under water with the sun `elevation` degrees up, a share of the day's: the water's pigment is
/// emitted radiance standing for the daylight it scatters back, so it falls as the light does. By day and through civil
/// twilight the web's ramp, a quarter of the day's at the end of it (`0.24 + 0.76 × daylight`, daylight from −6° to
/// 18°). After it the quarter falls with the twilight's real light (`naval_render::lights::twilight_lux`) once that is
/// under a full moon's (0.27 lux, about 8.5° down), which the game's night lifts to the same light the end of civil
/// twilight gives the scene (`naval_render::lights::candela_scale`): at the night preset's −28° it is a four-hundredth of
/// what it was, the water near black but for what lights it (it held the quarter all night until 2026-10-20, a teal fill
/// under any sky: `native/docs/lighting/night.md`, "Under water at night").
pub fn water_light_scale(elevation: f64) -> f64 {
    let daylight = smooth(elevation, -6.0, 18.0);
    let twilight = (f64::from(naval_render::lights::twilight_lux(elevation as f32))
        / f64::from(naval_render::lights::FULL_MOON_LUX))
    .min(1.0);
    0.24 * twilight + 0.76 * daylight
}

/// Whether `conditions` chose the clouds' base and depth (`naval_sky::CloudScene::base_chosen`, `depth_chosen`): a
/// genus brings its own (`resolve_clouds`), and the battle rail or the console may set either.
pub fn chosen_heights(conditions: &BattleConditions) -> (bool, bool) {
    let genus = matches!(conditions.cloud_type, Some(CloudChoice::Genus(_)));
    (
        genus || conditions.cloud_base.is_some(),
        genus || conditions.cloud_depth.is_some(),
    )
}

/// A cloud genus's typical layer (`CLOUD_GENERA`): base, depth, cover, base shadow, rain and lightning.
struct GenusInfo {
    base: f64,
    depth: f64,
    cover: f64,
    base_shadow: f64,
    rain: f64,
    lightning: f64,
}

fn genus_info(genus: CloudGenus) -> GenusInfo {
    let info = |base, depth, cover, base_shadow, rain, lightning| GenusInfo {
        base,
        depth,
        cover,
        base_shadow,
        rain,
        lightning,
    };
    match genus {
        CloudGenus::Cumulus => info(1100.0, 1900.0, 0.35, 0.2, 0.0, 0.0),
        CloudGenus::Cumulonimbus => info(800.0, 10_000.0, 0.25, 0.5, 0.35, 2.0),
        CloudGenus::Stratocumulus => info(800.0, 900.0, 0.8, 0.35, 0.0, 0.0),
        CloudGenus::Stratus => info(250.0, 450.0, 0.95, 0.3, 0.12, 0.0),
        CloudGenus::Nimbostratus => info(600.0, 3500.0, 1.0, 0.55, 0.55, 0.0),
        CloudGenus::Altocumulus => info(3500.0, 700.0, 0.75, 0.15, 0.0, 0.0),
        CloudGenus::Altostratus => info(4000.0, 1500.0, 1.0, 0.15, 0.0, 0.0),
    }
}

/// The high sheet's typical height and amount (`HIGH_CLOUDS`).
fn high_info(genus: HighCloudGenus) -> (f64, f64) {
    match genus {
        HighCloudGenus::Cirrus => (9000.0, 0.8),
        HighCloudGenus::Cirrostratus => (8500.0, 0.85),
        HighCloudGenus::Cirrocumulus => (7500.0, 0.75),
    }
}

/// The clouds a scene shows before any choice (`SceneClouds`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneClouds {
    pub coverage: f64,
    pub altitude: f64,
    pub thickness: f64,
    pub base_shadow: f64,
    pub horizon_coverage: f64,
}

/// `resolveClouds`: the layer and sheet `conditions` ask for over the scene's own clouds.
pub fn resolve_clouds(
    scene: SceneClouds,
    conditions: &BattleConditions,
) -> (SceneClouds, Option<CloudGenus>, Option<HighClouds>) {
    let genus = match conditions.cloud_type {
        Some(CloudChoice::Genus(genus)) => Some(genus),
        _ => None,
    };
    let info = genus.map(genus_info);
    let coverage = match (conditions.cloud_cover, &info) {
        (Some(cover), _) => (cover / 100.0).clamp(0.0, 1.0),
        (None, Some(info)) => info.cover,
        (None, None) => scene.coverage,
    };
    let layer = SceneClouds {
        coverage,
        altitude: conditions
            .cloud_base
            .or(info.as_ref().map(|i| i.base))
            .unwrap_or(scene.altitude),
        thickness: conditions
            .cloud_depth
            .or(info.as_ref().map(|i| i.depth))
            .unwrap_or(scene.thickness),
        base_shadow: info.as_ref().map_or(scene.base_shadow, |i| i.base_shadow),
        horizon_coverage: if info.is_some() {
            0.06 * coverage
        } else {
            scene.horizon_coverage
        },
    };
    let choice = conditions.high_cloud.unwrap_or(
        if conditions.high_cloud_cover.is_some() || conditions.high_cloud_base.is_some() {
            HighCloudChoice::Genus(HighCloudGenus::Cirrus)
        } else {
            HighCloudChoice::Auto
        },
    );
    let high = match choice {
        HighCloudChoice::Auto => None,
        HighCloudChoice::None | HighCloudChoice::Genus(_) => {
            let sheet = match choice {
                HighCloudChoice::Genus(genus) => genus,
                _ => HighCloudGenus::Cirrus,
            };
            let (altitude, amount) = high_info(sheet);
            Some(HighClouds {
                genus: sheet,
                amount: if choice == HighCloudChoice::None {
                    0.0
                } else {
                    conditions
                        .high_cloud_cover
                        .map_or(amount, |c| (c / 100.0).clamp(0.0, 1.0))
                },
                altitude: conditions.high_cloud_base.unwrap_or(altitude),
            })
        }
    };
    (layer, genus, high)
}

/// The storm's rain (0–1): 2.2 mm/h (`naval_clouds::weather::rain_rate`, 40 p² mm/h), the rate whose drops alone stop
/// the light the storm's visibility allows (its fog end, 8 km, which the lookouts see to): an extinction of
/// `naval_sky::haze::SIGHTING_DEPTH` / 8 km, 0.30 per km, from Marshall–Palmer's drops' cross-section
/// (`naval_clouds::weather::rain_extinction`). It was 0.9, 32 mm/h, whose own visibility is 1.5 km (2026-10-20,
/// `native/docs/clouds/weather.md`, "The storm's rain").
pub const STORM_RAIN: f64 = 0.236;

/// `battlePrecipitation`: rain (0–1) and lightning strikes per minute. The storm's rain is the rate whose own
/// visibility is the storm's 8 km (`STORM_RAIN`); its cells pour harder in their shafts (`naval_sim::clouds::weathers`).
pub fn battle_precipitation(weather: &str, conditions: &BattleConditions) -> (f64, f64) {
    let preset: (f64, f64) = match weather {
        "overcast" => (0.15, 0.0),
        "storm-clouds" => (STORM_RAIN, 3.0),
        _ => (0.0, 0.0),
    };
    let cover = conditions.cloud_cover.map_or(0.0, |c| c / 100.0);
    let gale = conditions.wind_speed.unwrap_or(0.0);
    if let Some(CloudChoice::Genus(genus)) = conditions.cloud_type {
        let info = genus_info(genus);
        let coverage = conditions.cloud_cover.map_or(info.cover, |_| cover);
        let own = (
            info.rain * smooth(coverage, 0.5, 0.95),
            info.lightning * smooth(coverage, 0.05, 0.2),
        );
        let driven = smooth(gale, 8.0, 18.0);
        return (
            preset.0.max((own.0 * (1.0 + 0.6 * driven)).min(1.0)),
            preset.1.max(own.1 * (1.0 + 0.5 * driven)),
        );
    }
    (
        preset
            .0
            .max(smooth(cover, 0.75, 1.0) * (0.25 + 0.65 * smooth(gale, 8.0, 18.0))),
        preset
            .1
            .max(smooth(cover, 0.9, 1.0) * smooth(gale, 14.0, 20.0) * 3.0),
    )
}

/// Share of each scene's authored forward sun haze that reaches the sky (`SUN_HAZE`).
pub const SUN_HAZE: f64 = 0.45;
/// The port's standing wind: 9 m/s toward 35° (`PORT_WIND`).
pub const PORT_WIND: (f64, f64) = (9.0, 35.0);
/// The port's ambient fill (`PORT_LIGHT.ambient`).
pub const PORT_AMBIENT: f64 = 1.75;
/// Sky and ground colours of the hemisphere fill every lit mesh takes (`MESH_FILL_COLORS`).
pub const MESH_FILL_COLORS: (&str, &str) = ("#dcebf2", "#65757e");

/// A map, time, weather and conditions: what the client picks before a battle (or the port).
#[derive(Clone, Debug, PartialEq)]
pub struct MapSelection {
    pub map: String,
    pub time_of_day: String,
    pub weather: String,
    pub conditions: BattleConditions,
    /// The port instead of a battle: the port's sheltered light, standing wind and short fog on this map's sea.
    pub port: bool,
    /// Metres between the spawn lines, which places the chart; `None` takes the map's own.
    pub spawn_distance: Option<f64>,
}

impl MapSelection {
    /// A battle on `map` at its own time and weather.
    pub fn battle(map: &str) -> Self {
        Self {
            map: map.to_owned(),
            time_of_day: "map".into(),
            weather: "map".into(),
            conditions: BattleConditions::default(),
            port: false,
            spawn_distance: None,
        }
    }

    /// The port on the North Atlantic's open sea.
    pub fn port() -> Self {
        Self {
            port: true,
            ..Self::battle(DEFAULT_MAP)
        }
    }

    /// Resolves the selection into what the sky, the sea, the land and the meshes read; `None` for an unknown map or
    /// preset.
    pub fn resolve(&self) -> Option<MapScene> {
        let map = map(&self.map)?;
        let port = self.port;
        // The port resolves its standing wind through the same sea and day model on the berth's map.
        let conditions = if port {
            BattleConditions {
                wind_speed: Some(PORT_WIND.0),
                ..BattleConditions::default()
            }
        } else {
            self.conditions
        };
        let (time, weather) = if port {
            ("map", "map")
        } else {
            (self.time_of_day.as_str(), self.weather.as_str())
        };
        let environment = battle_environment(map, time, weather, &conditions)?;
        let (rain, lightning) = if port {
            (0.0, 0.0)
        } else {
            battle_precipitation(weather, &conditions)
        };
        let authored = environment.sky;
        let port_sky = naval_sky::state::DEFAULT_SKY_SCENE;
        let scene_clouds = if port {
            SceneClouds {
                coverage: port_sky.clouds.coverage,
                altitude: port_sky.clouds.altitude,
                thickness: port_sky.clouds.thickness,
                horizon_coverage: port_sky.clouds.horizon_coverage,
                base_shadow: port_sky.clouds.base_shadow,
            }
        } else {
            SceneClouds {
                coverage: authored.coverage,
                altitude: authored.altitude,
                thickness: authored.thickness,
                horizon_coverage: environment.horizon_coverage,
                base_shadow: environment.cloud_shadow,
            }
        };
        let (clouds, genus, high) = resolve_clouds(scene_clouds, &conditions);
        let (base_chosen, depth_chosen) = chosen_heights(&conditions);
        let wind_direction = if port {
            PORT_WIND.1
        } else {
            conditions
                .wind_direction
                .unwrap_or(map.water.wind_direction)
        };
        let sky = SkyScene {
            sun_elevation: if port {
                port_sky.sun_elevation
            } else {
                authored.elevation
            },
            sun_azimuth: if port {
                port_sky.sun_azimuth
            } else {
                authored.azimuth
            },
            sun_intensity: if port {
                port_sky.sun_intensity
            } else {
                authored.intensity
            },
            moon_phase: conditions.moon.map_or(0.5, |moon| moon.phase),
            // The chart's top bears `map.bearing`, as for the sun.
            moon: conditions.moon.map(|moon| SkyAngles {
                elevation: moon.elevation,
                azimuth: (moon.azimuth + map.bearing).rem_euclid(360.0),
            }),
            atmosphere: if port {
                port_sky.atmosphere
            } else {
                AuthoredAtmosphere {
                    rayleigh: authored.rayleigh,
                    turbidity: authored.turbidity,
                    mie: authored.mie * SUN_HAZE,
                    mie_g: authored.mie_g,
                    multiple: authored.multiple,
                }
            },
            clouds: CloudScene {
                coverage: clouds.coverage,
                altitude: clouds.altitude,
                thickness: clouds.thickness,
                horizon_coverage: clouds.horizon_coverage,
                ambient: if port {
                    port_sky.clouds.ambient
                } else {
                    environment.cloud_ambient
                },
                base_shadow: clouds.base_shadow,
                wind_speed: environment.cloud_wind,
                wind_heading: ((90.0 - wind_direction) % 360.0 + 360.0) % 360.0,
                genus,
                high,
                fog: if port { 0.0 } else { environment.fog_banks },
                base_chosen,
                depth_chosen,
            },
            weather: WeatherScene {
                precipitation: rain,
                lightning,
            },
            port,
        };
        // The sea: the berth's map colours in port are the North Atlantic's.
        let water = if port {
            &calibration::NORTH_ATLANTIC
        } else {
            map.ocean_water()
        };
        let mut ocean = calibration::scene_parameters(water, environment.wind, port);
        if !port {
            let fog = &environment.fog;
            ocean.fog = FogParameters {
                color: srgb_hex(&fog.color),
                start: fog.start as f32,
                end: fog.end as f32,
                power: 1.4,
                sky_blend_distance: fog.sky_blend as f32,
            };
            // The water pigment is emitted radiance: night seas take the day's share of it.
            let fill = environment.water_light_scale as f32;
            for c in 0..3 {
                ocean.colors.water[c] *= fill;
                ocean.colors.transmission[c] *= fill;
            }
        }
        let spawn_distance = self.spawn_distance.unwrap_or(map.spawn_distance());
        Some(MapScene {
            map,
            port,
            sky,
            fog: ocean.fog,
            ocean,
            ambient: if port { PORT_AMBIENT } else { authored.ambient },
            wind: (environment.waves.wind_speed, wind_direction),
            style: map.surface_style(),
            terrain: if port { None } else { map.land.terrain.clone() },
            offset: OceanMap::custom_offset(spawn_distance),
            spawn_distance,
            environment,
        })
    }
}

/// A resolved map selection: everything the client hands to the sky, the sea, the land and the meshes.
#[derive(Clone, Debug)]
pub struct MapScene {
    pub map: &'static OceanMap,
    pub port: bool,
    /// For `naval_sky::Sky::apply` / `naval_clouds::GameSky::apply`.
    pub sky: SkyScene,
    /// For `naval_ocean::Ocean::new` (the fog within is the scene's: every material fogs with it).
    pub ocean: OceanParameters,
    /// The scene fog every material takes (`scene.fogNode`, owned by the ocean on the web).
    pub fog: FogParameters,
    /// The authored ambient before the meshes' share (`VisualEnvironment.ambient`): the hemisphere fill's intensity.
    pub ambient: f64,
    /// The sea's wind: speed (m/s) and direction (chart degrees from +X toward +Z).
    pub wind: (f64, f64),
    /// The land's palette.
    pub style: SurfaceStyle,
    /// The map's baked heightfield (`OceanMap.land.terrain`): `None`, open sea, since the land maps were removed; the
    /// port has none.
    pub terrain: Option<String>,
    /// World = chart + offset (x, z), for a staged battle at `spawn_distance`.
    pub offset: [f64; 2],
    pub spawn_distance: f64,
    /// `battleEnvironment`'s full result.
    pub environment: BattleEnvironment,
}

/// Shares of the celestial light lit meshes take: the sun's (or moon's) multiple, the hemisphere fill's multiple of
/// the authored ambient with a lightning `flash` (a multiple of the ambient, 0 for none), and the sky reflection's
/// intensity. The light as it is, by day and by night: the sun or moon at its own strength and the sky's environment as
/// the only ambient. The removed web game's `meshLightShares` filled shaded sides with the authored ambient (by day
/// below an intensity of 4, all of it by night with 1.5 of the moon), past what the sky gives; only a lightning flash
/// fills now ([`LIGHTNING_FILL`] × it, the web's `MESH_NIGHT_FILL`).
pub fn mesh_light_shares(_intensity: f64, _night: bool, flash: f64) -> (f64, f64, f64) {
    (1.0, LIGHTNING_FILL * flash, 1.0)
}

/// The hemisphere fill a lightning flash lends lit meshes, a multiple of the authored ambient per unit of flash
/// (`naval_ships::LIGHTNING_FILL`, the ships').
pub const LIGHTNING_FILL: f64 = 1.6;

#[cfg(test)]
mod tests {
    use super::*;

    /// The storm's rain alone stops the light its visibility allows: its drops' extinction is the game's calibration
    /// over the storm's fog end (8 km), the distance its lookouts see to.
    #[test]
    fn the_storms_rain_matches_its_visibility() {
        let map = map("north-atlantic").unwrap();
        let storm =
            battle_environment(map, "noon", "storm-clouds", &BattleConditions::default()).unwrap();
        let (rain, _) = battle_precipitation("storm-clouds", &BattleConditions::default());
        assert_eq!(rain, STORM_RAIN);
        let extinction =
            naval_clouds::weather::rain_extinction(naval_clouds::weather::rain_rate(rain));
        let own_visibility = naval_sky::haze::SIGHTING_DEPTH / extinction;
        assert!(
            (own_visibility / storm.fog.end - 1.0).abs() < 0.01,
            "{own_visibility} m against {} m",
            storm.fog.end
        );
    }

    /// A scene given its moon (a theatre's, by its date) draws it there at that phase, turned with the chart as the
    /// sun is (the North Atlantic's chart's top bears north); a scene given none keeps the full moon opposite the sun.
    #[test]
    fn a_moon_set_outright_reaches_the_sky() {
        let mut selection = MapSelection::battle("north-atlantic");
        let bearing = map("north-atlantic").unwrap().bearing;
        let authored = selection.resolve().unwrap().sky;
        assert_eq!((authored.moon_phase, authored.moon), (0.5, None));
        selection.conditions.moon = Some(Moon {
            elevation: 12.0,
            azimuth: 300.0,
            phase: 0.88,
        });
        let sky = selection.resolve().unwrap().sky;
        assert_eq!(sky.moon_phase, 0.88);
        let moon = sky.moon.unwrap();
        assert_eq!(moon.elevation, 12.0);
        assert!((moon.azimuth - (300.0 + bearing).rem_euclid(360.0)).abs() < 1e-9);
        // The port has no date.
        selection.port = true;
        assert_eq!(selection.resolve().unwrap().sky.moon, None);
    }

    /// A time preset's or a map's own sun lies on the day's arc through its hour, off it in azimuth by a constant.
    #[test]
    fn a_sun_is_found_on_the_days_arc() {
        for hour in [6.3, 7.574, 12.0, 15.0, 17.836, 19.574, 23.0, 2.5] {
            let sun = sun_at_hour(hour);
            let (found, offset) = hour_of_sun(sun.elevation, sun.azimuth);
            assert!((found - hour).abs() < 1e-6, "{hour}: {found}");
            assert!(offset.abs() < 1e-6, "{hour}: {offset}");
        }
        // The North Atlantic's own sun (48° up toward 235°) is the afternoon's, 8° round from the day's azimuth.
        let (hour, offset) = hour_of_sun(48.0, 235.0);
        let sun = sun_at_hour(hour);
        assert!((sun.elevation - 48.0).abs() < 1e-6 && hour > 12.0);
        assert!(((sun.azimuth + offset).rem_euclid(360.0) - 235.0).abs() < 1e-6);
        // Noon is 70° up; two hours past 16:00 the sun is on the horizon.
        assert!((sun_at_hour(12.0).elevation - 70.0).abs() < 1e-9);
        assert!(sun_at_hour(18.0).elevation.abs() < 1e-9);
    }

    /// The clouds drift with the map's wind, or with a wind the conditions give (a theatre scenario's).
    #[test]
    fn the_clouds_drift_with_the_wind_the_conditions_give() {
        let map_own = MapSelection::battle("north-atlantic").resolve().unwrap();
        let water = map("north-atlantic").unwrap().water.wind_direction;
        assert!((map_own.sky.clouds.wind_heading - (90.0 - water).rem_euclid(360.0)).abs() < 1e-9);
        let mut selection = MapSelection::battle("north-atlantic");
        // A wind from the north-west blows toward the south-east: 45° in the chart frame (x east, z south).
        selection.conditions.wind_direction = Some(320.0 + 90.0 - 360.0);
        let turned = selection.resolve().unwrap();
        assert!((turned.sky.clouds.wind_heading - 40.0).abs() < 1e-9);
        assert!((turned.wind.1 - 50.0).abs() < 1e-9);
    }

    /// The sun and moon at their own strength, and no fill but a lightning flash's, by day and by night.
    #[test]
    fn meshes_take_the_light_as_it_is() {
        for (intensity, night) in [(0.6, true), (0.0, false), (2.0, false), (6.6, false)] {
            assert_eq!(mesh_light_shares(intensity, night, 0.0), (1.0, 0.0, 1.0));
            assert!((mesh_light_shares(intensity, night, 0.5).1 - 0.8).abs() < 1e-12);
        }
    }
}
