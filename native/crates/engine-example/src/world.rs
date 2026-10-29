//! The scene: every engine layer built on one renderer, in the conditions `naval-conditions` resolves, and the effects
//! a host spawns through the effect crates' own inputs. What each layer needs from its host is in README.md.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use glam::{DVec3, Vec3};
use naval_clouds::{GameSky, Targets};
use naval_conditions::{MapScene, MapSelection};
use naval_effects::events::ShellInfo;
use naval_effects::tactical::ScreenSmoke;
use naval_effects::{CameraView, CombatEvent, DepthSource, EffectsLayer, Rgb};
use naval_ocean::{Ocean, QualityTier};
use naval_render::{
    BloomSettings, Camera, EyeAdaptationSettings, Renderer, SceneLayer, ShadowSettings, SkyMean,
    Sun,
};
use naval_sky::{SkyMeanLayer, SkyQuality};
use naval_terrain::textures::DETAIL_PHOTOS;
use naval_terrain::{TerrainLayer, TerrainLight, TerrainSettings};
use naval_water_effects::fixture::{WaterEvent, WaterEventKind};
use naval_water_effects::{EffectsFrame, GunBlast, WaterEffects};

use crate::coast;

/// The weathers the weather key steps through (`naval_conditions::weather_presets`, less the map's own).
pub const WEATHERS: [&str; 5] = ["clear", "partly-cloudy", "overcast", "fog", "storm-clouds"];

/// A smoke screen's life, seconds: it spreads, then thins away.
const SMOKE_LIFE_S: f64 = 90.0;

/// The conditions a host chooses: the hour, a weather and the wind that raises the sea (m/s; `None` keeps the
/// weather's own).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditions {
    pub hour: f64,
    pub weather: usize,
    pub wind: Option<f64>,
}

impl Conditions {
    /// The scene they resolve to on the North Atlantic's sea, its chart laid on the world (world = chart).
    pub fn scene(&self) -> Result<MapScene> {
        let mut selection = MapSelection::battle(naval_conditions::DEFAULT_MAP);
        selection.weather = WEATHERS[self.weather % WEATHERS.len()].into();
        selection.conditions.time_hours = Some(self.hour.rem_euclid(24.0));
        selection.conditions.wind_speed = self.wind;
        let mut scene = selection.resolve().context("the conditions resolve")?;
        scene.offset = [0.0, 0.0];
        Ok(scene)
    }
}

pub struct World {
    pub conditions: Conditions,
    pub scene: MapScene,
    /// The coast's heights, which the land layer draws (world = chart + the scene's offset).
    field: naval_terrain::Heightfield,
    sky: GameSky,
    mean: SkyMeanLayer,
    terrain: TerrainLayer,
    ocean: Ocean,
    water: WaterEffects,
    effects: EffectsLayer,
    /// What the host spawned since the last frame: the combat effects' events and the water's.
    events: Vec<CombatEvent>,
    splashes: Vec<WaterEvent>,
    /// The smoke screens laid, with the time each was laid.
    smoke: Vec<(ScreenSmoke, f64)>,
    sequence: u64,
    now: f64,
}

impl World {
    /// Every layer on `renderer`'s device, in `conditions`, over the synthetic release's coast.
    pub fn new(renderer: &mut Renderer, conditions: Conditions) -> Result<Self> {
        renderer.set_bloom(Some(BloomSettings::default()));
        renderer.set_eye_adaptation(EyeAdaptationSettings {
            enabled: true,
            ..Default::default()
        });
        renderer.set_sky_mean(SkyMean::Gpu);
        renderer.set_shadow_settings(ShadowSettings::for_quality("medium").context("shadows")?);
        let scene = conditions.scene()?;
        let field = coast::heightfield()?;
        let mut ctx = renderer.context();
        let (device, queue) = (ctx.device, ctx.queue);

        // The sky with its atmosphere, clouds and weather. No placement: the clouds lie flat about the camera (a host
        // with a weather field of its own lays them with `GameSky::placement`).
        let mut sky = GameSky::new_in(&mut ctx, SkyQuality::Medium, Targets::default())
            .map_err(|e| anyhow!("the sky's shaders: {e:?}"))?;
        sky.apply(queue, &scene.sky);
        sky.set_visibility(scene.ocean.fog.end);
        let environment = sky.sky.environment();
        let mean = SkyMeanLayer::new(&mut ctx, environment)
            .map_err(|e| anyhow!("the sky mean's shader: {e:?}"))?;
        let shadow = sky.clouds.shadow();
        let cloud_shadow = Some((&shadow.view, &shadow.sampler, &shadow.uniforms));

        // The sea, reading the opaque scene (the land meets it in shallows and foam) and the coast's shelter.
        let mut ocean = Ocean::new_in(
            &mut ctx,
            QualityTier::High,
            &environment.bind_group_layout,
            &environment.bind_group,
            f64::from(naval_render::camera::DEFAULT_FAR),
        )
        .map_err(|e| anyhow!("the ocean's shaders: {e:?}"))?;
        ocean.parameters = scene.ocean;
        ocean.read_scene_copies = true;
        ocean.set_cloud_shadow(device, cloud_shadow);
        ocean.set_shore(Some(coast::shore(
            &field,
            ocean.parameters.waves.wind_direction,
        )));
        ocean.set_shore_offset(scene.offset);
        let probe = ocean.underwater_probe().clone();

        // The land, with the detail photographs the host reads (the crate reads no file).
        let mut terrain = TerrainLayer::new(
            &mut ctx,
            &field,
            &scene,
            TerrainSettings::default(),
            &environment.bind_group,
        )
        .map_err(|e| anyhow!("the land's shaders: {e:?}"))?;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let photos = DETAIL_PHOTOS
            .iter()
            .map(|path| std::fs::read(root.join(path)).with_context(|| path.to_string()))
            .collect::<Result<Vec<_>>>()?;
        terrain
            .set_detail_photos(queue, [&photos[0], &photos[1], &photos[2], &photos[3]])
            .map_err(anyhow::Error::msg)?;
        terrain.set_cloud_shadow(Some(&shadow.bind_group));

        // The effects: the water's splashes and spray, then the combat gas, fire and smoke over them.
        let mut water = WaterEffects::new(&mut ctx)
            .map_err(|e| anyhow!("the water effects' shaders: {e:?}"))?;
        water.set_environment(
            device,
            &environment.view,
            &environment.sampler,
            &environment.uniforms,
        );
        let mut effects =
            EffectsLayer::new(&mut ctx).map_err(|e| anyhow!("the effects' shaders: {e:?}"))?;
        effects.depth_source = DepthSource::Backdrop;
        effects.set_environment(environment);
        effects.set_cloud_cover(Some(sky.clouds.cover()));
        effects.set_cloud_shadow(device, cloud_shadow);
        effects.set_splash_cover(Some(water.cover()));
        // The clouds stop at the sea's surface seen from under it.
        sky.set_underwater_probe(Some(probe));

        let mut world = Self {
            conditions,
            scene,
            field,
            sky,
            mean,
            terrain,
            ocean,
            water,
            effects,
            events: Vec::new(),
            splashes: Vec::new(),
            smoke: Vec::new(),
            sequence: 0,
            now: 0.0,
        };
        world.apply_fog();
        Ok(world)
    }

    /// Change the conditions: the sky, the sea and the fog every layer takes follow.
    pub fn set_conditions(&mut self, queue: &wgpu::Queue, conditions: Conditions) -> Result<()> {
        self.conditions = conditions;
        self.scene = conditions.scene()?;
        self.sky.apply(queue, &self.scene.sky);
        self.sky.set_visibility(self.scene.ocean.fog.end);
        // Changing the waves rebuilds the spectrum on the next update.
        self.ocean.parameters = self.scene.ocean;
        self.apply_fog();
        Ok(())
    }

    fn apply_fog(&mut self) {
        self.terrain.set_fog(self.scene.fog);
        self.effects.fog = naval_render::SceneFog::from(self.scene.ocean.fog).into();
    }

    /// A gun's blast, a fall of splashes, an HE burst, a shell striking the coast and a smoke screen, about `at` on
    /// the sea (`toward` the way the guns fire, level). Each is the effect crates' own input, as a host's simulation
    /// would send it: [`CombatEvent`]s for the gas, fire and smoke, [`WaterEvent`]s for the water, a [`ScreenSmoke`]
    /// for the screen.
    pub fn spawn(&mut self, at: DVec3, toward: DVec3) {
        let toward = DVec3::new(toward.x, 0.0, toward.z).normalize_or(DVec3::NEG_Z);
        let across = DVec3::new(-toward.z, 0.0, toward.x);
        let coast = self.land_along(at, toward);
        let mut next = || {
            self.sequence += 1;
            self.sequence
        };
        let shell = |id: u64, velocity: DVec3, kind: &str| ShellInfo {
            id: id as i64,
            caliber_m: 0.38,
            velocity: velocity.to_array(),
            shell_type: Some(kind.into()),
            explosive_kg: None,
        };
        // A twin turret's salvo, 14 m over the sea (its smoke and flash, and the spray its blast tears off the sea).
        for barrel in [-1.0, 1.0] {
            let sequence = next();
            let muzzle = at + across * (1.2 * barrel) + DVec3::Y * 14.0;
            self.events.push(CombatEvent {
                sequence,
                tick: sequence,
                kind: "shot".into(),
                ship_id: "example".into(),
                position: muzzle.to_array(),
                shell: Some(shell(sequence, toward * 820.0 + DVec3::Y * 30.0, "AP")),
                ..Default::default()
            });
        }
        // Three shells falling short and over, 600 m out.
        for (k, offset) in [(-1.0, -40.0), (0.0, 30.0), (1.0, 80.0)] {
            let sequence = next();
            let p = at + toward * (600.0 + offset) + across * (45.0 * k);
            self.splashes.push(WaterEvent {
                sequence,
                kind: WaterEventKind::Splash,
                position: [p.x, 0.0, p.z],
                velocity: Some((toward * 300.0 - DVec3::Y * 450.0).to_array()),
                caliber_m: Some(0.38),
                shell_id: Some(sequence as i64),
            });
        }
        // An HE shell bursting 25 m up, 350 m out.
        let sequence = next();
        let p = at + toward * 350.0 - across * 120.0 + DVec3::Y * 25.0;
        self.events.push(CombatEvent {
            sequence,
            tick: sequence,
            kind: "burst".into(),
            position: p.to_array(),
            shell: Some(shell(sequence, toward * 500.0 - DVec3::Y * 200.0, "HE")),
            detonation: Some(true),
            ..Default::default()
        });
        // A shell striking the coast (a stop that is no ship's): where the line ahead meets the land, if it does.
        if let Some(hit) = coast {
            let sequence = next();
            self.events.push(CombatEvent {
                sequence,
                tick: sequence,
                kind: "stopped".into(),
                position: hit.to_array(),
                shell: Some(shell(sequence, toward * 450.0 - DVec3::Y * 250.0, "HE")),
                ..Default::default()
            });
        }
        // A smoke screen laid 250 m off, across the line of fire.
        let id = next() as i64;
        let position = at + toward * 250.0 + across * 200.0;
        self.smoke.push((
            ScreenSmoke {
                id,
                position: DVec3::new(position.x, 0.0, position.z),
                radius: 40.0,
                height: 20.0,
                density: 1.0,
                age: 0.0,
            },
            self.now,
        ));
    }

    /// The first land along the level line from `at` toward `toward`, within 12 km, on the field the layer draws.
    fn land_along(&self, at: DVec3, toward: DVec3) -> Option<DVec3> {
        let field = &self.field;
        (1..300)
            .map(|k| at + toward * (f64::from(k) * 40.0))
            .find_map(|p| {
                let h = field.height(p.x - self.scene.offset[0], p.z - self.scene.offset[1]);
                (h > 2.0).then(|| DVec3::new(p.x, h, p.z))
            })
    }

    /// Step the scene `dt` seconds for a frame of `camera` at `width` × `height`: the light every layer takes from
    /// the sky, and the effects.
    pub fn advance(
        &mut self,
        renderer: &mut Renderer,
        camera: &Camera,
        (width, height): (u32, u32),
        dt: f64,
    ) {
        self.now += dt;
        let light = self.sky.light();
        let flash = self.sky.lightning().flash;
        // The sun's shadow maps centre where the camera looks.
        renderer.set_sun(Some(Sun::new(light.direction, camera.target)));
        let ambient = self.scene.ambient;
        self.terrain.set_light(&TerrainLight {
            light,
            ambient: ambient as f32,
            flash,
        });

        // The effects' light: the sky's sun or moon, its ambient lifted by a lightning flash.
        let (color, direction) = (light.color.as_dvec3(), light.direction.as_dvec3());
        let intensity = f64::from(light.intensity);
        let flash = f64::from(flash);
        let lit = naval_water_effects::EffectLighting::from_sky(
            direction,
            color,
            intensity,
            light.night,
            ambient,
            false,
        );
        let (wind, toward) = (self.scene.wind.0, self.scene.wind.1.to_radians());
        let lighting = &mut self.effects.effects.lighting;
        lighting.sun_direction = direction;
        lighting.daylight = lit.daylight;
        lighting.set_illumination(
            Rgb {
                r: color.x,
                g: color.y,
                b: color.z,
            },
            intensity,
            ambient * (1.0 + flash),
        );
        lighting.flash = flash;
        lighting.set_wind(wind, toward);
        let water = &mut self.water.lighting;
        water.set_sun(direction, lit.daylight);
        water.set_illumination(color, intensity, ambient * (1.0 + flash));
        water.flash = flash;
        water.set_wind(wind, toward);

        // The smoke screens the host keeps: each spreads and thins with its age, gone after its life.
        let now = self.now;
        self.smoke.retain(|(_, laid)| now - laid < SMOKE_LIFE_S);
        self.effects.effects.tactical.smoke = self
            .smoke
            .iter()
            .map(|(screen, laid)| {
                let age = now - laid;
                ScreenSmoke {
                    age,
                    radius: screen.radius + 2.0 * age,
                    height: screen.height + 0.6 * age,
                    density: (1.0 - age / SMOKE_LIFE_S).clamp(0.0, 1.0),
                    ..*screen
                }
            })
            .collect();

        let view = CameraView::from_render(camera, width, height);
        let events = std::mem::take(&mut self.events);
        self.effects
            .effects
            .update(dt, self.now, &view, &events, &[]);
        // The turrets that fired lay their sheets of spray on the sea under the guns.
        let blasts: Vec<GunBlast> = self
            .effects
            .effects
            .turret_blasts()
            .iter()
            .map(|blast| GunBlast {
                muzzle: blast.muzzle.to_array(),
                bore: blast.bore.to_array(),
                caliber_m: blast.caliber_m,
                barrels: blast.barrels,
                sequence: blast.sequence,
            })
            .collect();
        self.water.gun_blasts(&blasts);
        let splashes = std::mem::take(&mut self.splashes);
        self.water.update(&EffectsFrame {
            dt,
            camera: view,
            ships: &[],
            events: &splashes,
            sea: None,
            waves: self.ocean.parameters.waves.significant_height,
            crest: None,
            lighting: self.water.lighting,
        });
    }

    /// The layers in draw order: the sky first (its prepare refilters the environment the others read this frame),
    /// its mean for the eye, the land, the sea over the opaque scene, then the transparent effects.
    pub fn layers(&mut self) -> [&mut dyn SceneLayer; 6] {
        [
            &mut self.sky,
            &mut self.mean,
            &mut self.terrain,
            &mut self.ocean,
            &mut self.water,
            &mut self.effects,
        ]
    }
}

/// Where effects spawn for a view from `eye` along `forward`: where it meets the sea, at most 2 km off (and at least
/// 100 m), or 300 m ahead on the sea when it looks above the horizon.
pub fn ahead(eye: Vec3, forward: Vec3) -> DVec3 {
    let level = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
    let distance = if forward.y < -0.01 {
        let horizontal = Vec3::new(forward.x, 0.0, forward.z).length();
        (eye.y / -forward.y * horizontal).clamp(100.0, 2000.0)
    } else {
        300.0
    };
    let p = eye + level * distance;
    DVec3::new(f64::from(p.x), 0.0, f64::from(p.z))
}
