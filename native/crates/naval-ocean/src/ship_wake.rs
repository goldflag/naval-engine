//! Every wake the surface shades, behind one facade: a port of the web game's `ShipWake.ts` with the realistic trail
//! (`OceanRealism.wake`, on in the game). It drives the wake field's bow and stern generators for the nearest hulls, the
//! fleet's trail foam and slick ([`crate::trail`]), the torpedo tracks ([`crate::torpedo_tracks`]), the analytic bow waves
//! ([`crate::bow_waves`]) and the hull contact foam ([`crate::hull_foam`]); paints the trail and torpedo atlases on the
//! GPU ([`crate::stamps`]); and packs every system's arrays into the one uniform buffer `shaders/wake.wgsl` reads.
//!
//! Per frame: [`ShipWake::update`] with the drawn hulls, this frame's water events and torpedoes and the camera (CPU),
//! then [`ShipWake::encode`] on the frame's encoder (the field's steps, atlas repaints, the uniform upload). The ocean
//! does the second from its `prepare`.

use crate::bow_waves::{BowShip, BowWaves};
use crate::hull_foam::{ContactShip, HULL_FOAM_SLOTS, HullContactFoam, STRIDE};
use crate::hull_profile::WaterlineProfile;
use crate::oil::{OIL_FAR_EXTENT, OIL_NEAR_EXTENT, OIL_RESOLUTION, OIL_SLOTS, OIL_TILES};
use crate::splash_foam::{charge_scale, splash_scale};
use crate::stamps::{AtlasKind, StampPainter};
use crate::torpedo_tracks::{TorpedoTracks, TrackTorpedo};
use crate::trail::{ATLAS_CAPACITY, FleetTrails, TILES, TrailShip, WakeHull};
use crate::wake::WakeField;
use crate::wake::WakeFieldPipelines;
use crate::wake::generators::GeneratorOptions;
use crate::wake::impulses::{Strike, WakeImpulse, strike_impulse};
use naval_render::{MakePipelines, PipelineId, PipelineRegistry, RenderContext, ShaderError};

/// Header vec4s of the `Wake` uniform (`shaders/wake.wgsl`), then its arrays.
const HEADER: usize = 15;
const TILE_ROWS: usize = 5 * ATLAS_CAPACITY;
const BOW_ROWS: usize = crate::bow_waves::BOW_SLOT_ROWS * crate::bow_waves::BOW_WAVE_SLOTS;
const HULL_ROWS: usize = HULL_FOAM_SLOTS * STRIDE;
/// The oil's header vec4s after the hulls' (`Wake.oil`, `oil_reach`, `oil_flow`), then its slots' rows
/// (`crate::oil::OilRows`, section-major).
const OIL_HEADER: usize = 3;
const OIL_ROWS: usize = 5 * OIL_SLOTS;
/// The splashes' deposits' rows ([`crate::splash_foam::SplashDeposits::rows`]), then the foam field's
/// (`Wake.foam_state`, `Wake.sim`, `Wake.sim_grid`, [`crate::foam_sim::FoamSim::rows`]), last.
const FOAM_ROWS: usize = crate::splash_foam::SPLASH_ROWS + 3;
/// The `Wake` uniform's size in vec4s.
pub const WAKE_VEC4S: usize =
    HEADER + TILE_ROWS + BOW_ROWS + HULL_ROWS + OIL_HEADER + OIL_ROWS + FOAM_ROWS;
/// The wake field spans this many metres around the focus hull, keeping a 250 m hull's whole trail.
pub const WORLD_SIZE: f64 = 1536.0;
/// Hulls within this distance (each axis) of the focus get the field's generators, the nearest eight of them.
const GENERATOR_REACH: f64 = 900.0;
const GENERATOR_HULLS: usize = 8;
/// A re-lay carries a wake field by translation only where the two charts turn its water by no more than this
/// (radians; half a degree: under 3 m at 300 m from her hull, 7 m at the field's edge).
pub const RELAY_TURN: f64 = 0.0087;
/// A hull's own waves break in the field along her sides: within this many beams of the line from her bow and stern
/// generators to her middle (a Bismarck's, at full speed, all within 1.1 beams; a destroyer's not at all).
pub const BREAK_REACH: f64 = 1.25;

/// How much of a hull's wake reaches the surface with her origin at `y` (m; below 0 a diving submarine): all of it at
/// the surface, none from 3 m down. Her waves, trail, bow wave and the foam field's churn read her speed times this.
pub fn surfaced(y: f64) -> f64 {
    (1.0 + y / 3.0).clamp(0.0, 1.0)
}

/// A drawn hull as the wakes read it.
#[derive(Clone, Copy, Debug)]
pub struct WakeShip<'a> {
    pub id: &'a str,
    /// World position of the hull's origin (y below 0: a diving submarine).
    pub position: [f64; 3],
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
    /// Speed along the bow (m/s).
    pub speed: f64,
    pub length: f64,
    pub beam: f64,
    pub draft: f64,
    pub center_x: f64,
    pub center_z: f64,
    pub forward_speed: f64,
    pub reverse_speed: f64,
    /// The hull's waterline profile, for its contact foam; none draws no line.
    pub profile: Option<&'a WaterlineProfile>,
    /// The oil she is losing from holed bunkers (m³/s, `ShipVisual::oil_leak_m3s`): the slick she trails
    /// ([`crate::oil`]); 0 for none.
    pub oil_leak_m3s: f64,
}

/// A splash (or aircraft crash, or a torpedo's entry) that leaves a deposit of white water
/// ([`crate::splash_foam`]), sized by its calibre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WakeSplash {
    pub sequence: u64,
    pub x: f64,
    pub z: f64,
    pub caliber_m: f64,
}

/// The camera the wakes read: where it is, which way it looks, its vertical field of view (radians), the projection's
/// vertical scale (`projectionMatrix[5]`, which grows with magnification) and the drawing buffer's height in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WakeCamera {
    pub eye: [f64; 3],
    pub forward: [f64; 3],
    pub fov_y: f64,
    pub zoom: f64,
    pub drawing_height: f64,
}

pub struct WakeInputs<'a> {
    /// The focus hull first.
    pub ships: &'a [WakeShip<'a>],
    pub dt: f64,
    pub splashes: &'a [WakeSplash],
    pub torpedoes: &'a [TrackTorpedo],
    pub camera: WakeCamera,
    /// Significant wave height of the sea on show (m).
    pub sea_height: f64,
}

/// What the surface binds of the wakes (group 1, bindings 5–10, and the oil atlas at 16).
pub struct WakeBindings<'a> {
    pub uniforms: &'a wgpu::Buffer,
    pub displays: [&'a wgpu::TextureView; 2],
    pub trail_atlas: &'a wgpu::TextureView,
    pub torpedo_field: &'a wgpu::TextureView,
    /// The oil slicks' atlas ([`crate::oil`], `shaders/oil.wgsl`).
    pub oil_atlas: &'a wgpu::TextureView,
    /// The foam field's two states, an array ([`crate::foam_sim`]; binding 18).
    pub foam_sim: &'a wgpu::TextureView,
}

/// A wake field kept stepping about a hull the view has left for one beyond the field ([`ShipWake::update`]): the
/// waves she raises are still about her when the view comes back.
struct Standby {
    field: WakeField,
    /// The hull it follows.
    ship: String,
    /// Its generators (hull id, bow, stern), as [`ShipWake`]'s own.
    generators: Vec<(String, u32, u32)>,
}

/// What [`ShipWake::encode`] does to the field the focus took before it steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Taken {
    #[default]
    Kept,
    /// The standby's, taken for another place: its waves calmed (new textures).
    Calm,
    /// None yet: one is made beside the standby.
    Made,
}

pub struct ShipWake {
    /// The wake field about the focus hull, which the surface draws.
    pub field: Option<WakeField>,
    /// The hull `field` follows.
    anchor: Option<String>,
    standby: Option<Standby>,
    taken: Taken,
    /// What struck the sea while the focus's field was yet to be made ([`Taken::Made`]), laid on it once it is.
    waiting: Vec<WakeImpulse>,
    pub trails: FleetTrails,
    pub torpedoes: TorpedoTracks,
    pub bow_waves: BowWaves,
    pub hull_foam: HullContactFoam,
    trail_painter: StampPainter,
    torpedo_painter: StampPainter,
    oil_painter: StampPainter,
    uniforms: wgpu::Buffer,
    calm_displays: [wgpu::TextureView; 2],
    generators: Vec<(String, u32, u32)>,
    event_sequence: u64,
    center: [f64; 2],
    dt: f64,
    /// Moves when the textures the surface binds are remade (a field reset), so it knows to rebind.
    pub version: u64,
    /// The foam field ([`crate::foam_sim`]): none on a tier without one ([`ShipWake::set_foam_grid`]).
    pub foam_sim: crate::foam_sim::FoamSim,
    /// Where its window goes and the sterns that feed it this frame (from [`ShipWake::update`]).
    sim_centre: Option<[f64; 2]>,
    sim_sterns: Vec<crate::foam_sim::SimStern>,
    /// What the surface binds while there is no foam field: a calm array.
    calm_sim: wgpu::TextureView,
}

/// The wakes' pipelines on the renderer's registry ([`ShipWake::register_pipelines`]).
#[derive(Clone, Copy, Debug)]
pub struct ShipWakePipelines {
    field: Option<WakeFieldPipelines>,
    trail: PipelineId,
    torpedo: PipelineId,
    oil: PipelineId,
    foam_sim: crate::foam_sim::FoamSimPipelines,
}

impl ShipWake {
    /// Wakes for a wake field of `resolution`² cells (0: no field, a calm wake, as the Low tier) whose finest water-mesh
    /// vertices are `mesh_spacing` metres apart.
    pub fn new(device: &wgpu::Device, resolution: u32, mesh_spacing: f64) -> Self {
        Self::build(device, resolution, mesh_spacing, MakePipelines::Now)
    }

    /// Builds the field's and both painters' pipelines through the renderer's registry, as [`ShipWake::new`] builds
    /// them. The ocean's files must be registered first (`crate::shaders::register`).
    pub fn register_pipelines(
        &self,
        ctx: &mut RenderContext<'_>,
    ) -> Result<ShipWakePipelines, ShaderError> {
        Ok(ShipWakePipelines {
            field: self
                .field
                .as_ref()
                .map(|field| field.register_pipelines(ctx))
                .transpose()?,
            trail: self.trail_painter.register_pipeline(ctx)?,
            torpedo: self.torpedo_painter.register_pipeline(ctx)?,
            oil: self.oil_painter.register_pipeline(ctx)?,
            foam_sim: self.foam_sim.register_pipelines(ctx)?,
        })
    }

    /// Takes the registry's current pipelines in place of the wakes' own; whether any changed.
    pub fn adopt_pipelines(
        &mut self,
        registry: &PipelineRegistry,
        ids: &ShipWakePipelines,
    ) -> bool {
        let mut changed = false;
        if let Some(ids) = &ids.field {
            for field in self
                .field
                .iter_mut()
                .chain(self.standby.iter_mut().map(|s| &mut s.field))
            {
                changed |= field.adopt_pipelines(registry, ids);
            }
        }
        changed |= self.trail_painter.adopt_pipeline(registry, ids.trail);
        changed |= self.torpedo_painter.adopt_pipeline(registry, ids.torpedo);
        changed |= self.oil_painter.adopt_pipeline(registry, ids.oil);
        changed |= self.foam_sim.adopt_pipelines(registry, &ids.foam_sim);
        changed
    }

    pub(crate) fn build(
        device: &wgpu::Device,
        resolution: u32,
        mesh_spacing: f64,
        make: MakePipelines<'_>,
    ) -> Self {
        let field = (resolution > 0).then(|| {
            let mut field = WakeField::build(device, resolution, WORLD_SIZE, make);
            field.friction = 0.065;
            // Hull waves at full speed reach slopes of about 0.02 over the field's 12 m breaking baseline; this lets the
            // bow and stern shoulders break without foaming the whole wedge.
            field.foam_break_threshold = 0.015;
            field.foam_strength = 1.2;
            field.foam_lifetime = 9.0;
            field
        });
        // Trail foam is the game's own and stays on when the tier runs no wake field.
        let trail_resolution = if resolution > 0 {
            resolution.min(256)
        } else {
            128
        };
        let trails = FleetTrails::new(trail_resolution);
        let trail_painter = StampPainter::build(
            device,
            trail_resolution,
            TILES as u32,
            AtlasKind::Trails,
            make,
        );
        // A bubble track is a few metres wide: finer than any fleet wake cell.
        let torpedoes = TorpedoTracks::new(1024);
        let torpedo_painter = StampPainter::build(device, 1024, 1, AtlasKind::Torpedoes, make);
        // The oil slicks keep their own atlas on every tier: a leaking hull is rare, and her slick is the game's own.
        let oil_painter = StampPainter::build(
            device,
            OIL_RESOLUTION,
            OIL_TILES as u32,
            AtlasKind::Oil,
            make,
        );
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ship wake"),
            size: (WAKE_VEC4S * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let calm = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let mut bow_waves = BowWaves::default();
        bow_waves.vertex_spacing = mesh_spacing;
        Self {
            field,
            trails,
            torpedoes,
            bow_waves,
            hull_foam: HullContactFoam::default(),
            trail_painter,
            torpedo_painter,
            oil_painter,
            uniforms,
            calm_displays: [calm("calm wake 0"), calm("calm wake 1")],
            anchor: None,
            standby: None,
            taken: Taken::Kept,
            waiting: Vec::new(),
            generators: Vec::new(),
            event_sequence: 0,
            center: [0.0; 2],
            dt: 0.0,
            version: 0,
            foam_sim: crate::foam_sim::FoamSim::build(device, None, make),
            sim_centre: None,
            sim_sterns: Vec::new(),
            calm_sim: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("calm foam sim"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                }),
        }
    }

    /// What the surface binds.
    pub fn bindings(&self) -> WakeBindings<'_> {
        let displays = match &self.field {
            Some(field) => field.display_views(),
            None => [&self.calm_displays[0], &self.calm_displays[1]],
        };
        WakeBindings {
            uniforms: &self.uniforms,
            displays,
            trail_atlas: self.trail_painter.view(),
            torpedo_field: self.torpedo_painter.view(),
            oil_atlas: self.oil_painter.view(),
            foam_sim: self.foam_sim.states().unwrap_or(&self.calm_sim),
        }
    }

    /// Whether any oil slick lies on the water in a slot the surface draws (it then draws its oil variant).
    pub fn oil_shown(&self) -> bool {
        self.trails.oil_shown()
    }

    /// The surface's drift that carries the oil, from the wind the sea is drawn with: toward `direction` (radians from
    /// +x toward +z, `WaveParameters::wind_direction`) at [`crate::oil::DRIFT_SHARE`] of `speed` (m/s).
    pub fn set_wind(&mut self, speed: f64, direction: f64) {
        let drift = crate::oil::DRIFT_SHARE * speed.max(0.0);
        let (z, x) = direction.sin_cos();
        self.trails.oil_drift = if drift.is_finite() && x.is_finite() && z.is_finite() {
            [drift * x, drift * z]
        } else {
            [0.0; 2]
        };
    }

    /// Size the foam field for a tier ([`crate::foam_sim::FoamGrid::for_tier`]; none: the surface draws all the foam).
    pub fn set_foam_grid(&mut self, grid: Option<crate::foam_sim::FoamGrid>) {
        self.foam_sim.set_grid(grid);
        self.version += 1;
    }

    /// The CPU half of a frame (`ShipWake.update`).
    pub fn update(&mut self, inputs: &WakeInputs<'_>) {
        let dt = inputs.dt.max(0.0);
        self.dt = dt;
        let fresh: Vec<&WakeSplash> = inputs
            .splashes
            .iter()
            .filter(|s| s.sequence > self.event_sequence)
            .collect();
        for splash in &fresh {
            self.event_sequence = self.event_sequence.max(splash.sequence);
        }
        if let Some(focus) = inputs.ships.first() {
            self.center = [focus.position[0], focus.position[2]];
            self.follow(focus.id);
        }
        // The standby follows its hull while she is drawn. Once she is not it is let go, or taken calm for the focus
        // when it waits for a field of its own (the view left a hull as she went).
        let kept = self.standby.as_ref().and_then(|standby| {
            inputs
                .ships
                .iter()
                .find(|s| s.id == standby.ship)
                .map(|s| [s.position[0], s.position[2]])
        });
        if kept.is_none()
            && let Some(standby) = self.standby.take()
            && self.field.is_none()
        {
            let mut field = standby.field;
            field.restart_at(self.center[0], self.center[1]);
            self.field = Some(field);
            self.generators = standby.generators;
            self.taken = Taken::Calm;
            self.waiting.clear();
        }
        if let Some(field) = &mut self.field {
            field.set_center(self.center[0], self.center[1]);
        }
        // Drawing favours the focus and nearest hulls; every hull retains its trail history independently of slots.
        let span = |ship: &WakeShip<'_>| {
            (ship.position[0] - self.center[0]).hypot(ship.position[2] - self.center[1])
        };
        let mut ships: Vec<WakeShip<'_>> = inputs.ships.to_vec();
        if ships.len() > ATLAS_CAPACITY {
            let mut rest = ships.split_off(1);
            rest.sort_by(|a, b| span(a).total_cmp(&span(b)));
            ships.extend(rest);
        }
        if let Some(field) = &mut self.field {
            raise_waves(field, &mut self.generators, self.center, &ships);
        }
        if let (Some(standby), Some(centre)) = (&mut self.standby, kept) {
            standby.field.set_center(centre[0], centre[1]);
            raise_waves(&mut standby.field, &mut standby.generators, centre, &ships);
        }
        let camera = inputs.camera;
        let splashes: Vec<(f64, f64, f64)> = fresh
            .iter()
            .map(|s| (s.x, s.z, splash_scale(s.caliber_m)))
            .collect();
        let trail_ships: Vec<TrailShip<'_>> = ships
            .iter()
            .map(|s| TrailShip {
                id: s.id,
                position: s.position,
                heading: s.heading,
                speed: s.speed,
                hull: WakeHull {
                    length: s.length,
                    beam: s.beam,
                    forward_speed: s.forward_speed,
                    center_x: s.center_x,
                    center_z: s.center_z,
                },
                reverse_speed: s.reverse_speed,
                oil_leak_m3s: s.oil_leak_m3s,
            })
            .collect();
        self.trails
            .update(&trail_ships, dt, &splashes, Some((camera.eye, camera.zoom)));
        // The foam field: its window astern of the focus hull (or about the water the view looks at, far from her), fed
        // by every stern under way at the surface.
        self.sim_centre = None;
        self.sim_sterns.clear();
        if let Some(grid) = self.foam_sim.grid() {
            use crate::foam_sim::{place_window, stern_source, window_centre};
            let hull = inputs.ships.first().map(|s| {
                window_centre(s.position, s.heading, s.speed, s.forward_speed, grid.edge())
            });
            self.sim_centre = place_window(hull, camera.eye, camera.forward, grid.half());
            self.sim_sterns
                .extend(ships.iter().filter_map(stern_source));
        }
        let torpedo_camera = Some((camera.eye, camera.forward, camera.zoom));
        self.torpedoes
            .update(inputs.torpedoes, dt, self.center, torpedo_camera);
        let bow_ships: Vec<BowShip<'_>> = ships
            .iter()
            .map(|s| BowShip {
                id: s.id,
                position: s.position,
                heading: s.heading,
                speed: s.speed,
                length: s.length,
                beam: s.beam,
                draft: s.draft,
                center_x: s.center_x,
                center_z: s.center_z,
                forward_speed: s.forward_speed,
                profile: s.profile,
            })
            .collect();
        self.bow_waves.update(
            &bow_ships,
            dt,
            Some((camera.eye, camera.fov_y)),
            camera.drawing_height,
        );
        self.hull_foam.sea_height = inputs.sea_height;
        self.hull_foam.wash_width = self.bow_waves.tuning.wash_width;
        let contact: Vec<ContactShip<'_>> = ships
            .iter()
            .filter_map(|s| {
                Some(ContactShip {
                    id: s.id,
                    position: s.position,
                    heading: s.heading,
                    pitch: s.pitch,
                    roll: s.roll,
                    speed: s.speed,
                    beam: s.beam,
                    draft: s.draft,
                    forward_speed: s.forward_speed,
                    profile: s.profile?,
                })
            })
            .collect();
        self.hull_foam.update(
            &contact,
            dt,
            Some((camera.eye, camera.fov_y)),
            camera.drawing_height,
        );
    }

    /// The field the focus hull `id` is drawn with. One already raising her waves follows her on; else the standby,
    /// when it follows her or raises her waves; else the standby (or a new one, beside it) is taken for her place
    /// calm. Whichever field the view leaves is kept about the hull it followed, so going back to her finds her waves.
    fn follow(&mut self, id: &str) {
        if self.anchor.as_deref() == Some(id) || (self.field.is_none() && self.standby.is_none()) {
            return;
        }
        let raises =
            |generators: &[(String, u32, u32)]| generators.iter().any(|(ship, ..)| ship == id);
        if self.field.is_some() && (self.anchor.is_none() || raises(&self.generators)) {
            self.anchor = Some(id.to_owned());
            return;
        }
        let left = self
            .field
            .take()
            .zip(self.anchor.take())
            .map(|(field, ship)| Standby {
                field,
                ship,
                generators: std::mem::take(&mut self.generators),
            });
        match self.standby.take() {
            Some(standby) => {
                let theirs = standby.ship == id || raises(&standby.generators);
                let mut field = standby.field;
                if !theirs {
                    field.restart_at(self.center[0], self.center[1]);
                }
                self.field = Some(field);
                self.generators = standby.generators;
                self.taken = if theirs { Taken::Kept } else { Taken::Calm };
            }
            None => {
                self.generators.clear();
                self.taken = Taken::Made;
            }
        }
        self.standby = left;
        self.anchor = Some(id.to_owned());
        self.version += 1;
    }

    /// Ships that come aboard underway (a theatre scene, cruise or battle) lay the trails they have been leaving
    /// (`ShipWake.layTrails`): `seconds` of trail along each hull's present course and speed, a step at a time (2.5 s,
    /// or 90 m of the fastest hull's run), from one pose a step before the history opens. Only the trails; the wake
    /// field's swell and the bow waves build as they would.
    pub fn lay_trails(&mut self, ships: &[WakeShip<'_>], seconds: f64, camera: &WakeCamera) {
        let fastest = ships.iter().map(|s| s.speed.abs()).fold(0.0, f64::max);
        if seconds <= 0.0 || fastest <= 0.15 || !seconds.is_finite() {
            return;
        }
        let step = 2.5f64.min(90.0 / fastest);
        let steps = (seconds / step).ceil() as u64;
        for n in (0..=steps + 1).rev() {
            let ago = n as f64 * step;
            let past: Vec<TrailShip<'_>> = ships
                .iter()
                .map(|s| {
                    let run = s.speed * ago;
                    TrailShip {
                        id: s.id,
                        position: [
                            s.position[0] - s.heading.sin() * run,
                            s.position[1],
                            s.position[2] + s.heading.cos() * run,
                        ],
                        heading: s.heading,
                        speed: s.speed,
                        hull: WakeHull {
                            length: s.length,
                            beam: s.beam,
                            forward_speed: s.forward_speed,
                            center_x: s.center_x,
                            center_z: s.center_z,
                        },
                        reverse_speed: s.reverse_speed,
                        // A hull that comes aboard leaking has been leaking as she is.
                        oil_leak_m3s: s.oil_leak_m3s,
                    }
                })
                .collect();
            self.trails
                .update(&past, step, &[], Some((camera.eye, camera.zoom)));
        }
    }

    /// The GPU half of a frame: the wake field's steps, atlas repaints and the uniform upload, on `encoder`.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        self.encode_timed(
            device,
            &mut naval_render::UploadSink::Queue(queue),
            encoder,
            None,
        );
    }

    /// [`ShipWake::encode`], the wake field's steps timed by the renderer core's timer, every write through `queue`
    /// (the renderer core's staging belt in a layer's `prepare`).
    pub fn encode_timed(
        &mut self,
        device: &wgpu::Device,
        queue: &mut naval_render::UploadSink<'_>,
        encoder: &mut wgpu::CommandEncoder,
        timer: Option<&naval_render::GpuTimer>,
    ) {
        match std::mem::take(&mut self.taken) {
            Taken::Kept => {}
            Taken::Calm => {
                if let Some(field) = &mut self.field {
                    field.calm_textures(device);
                }
                self.version += 1;
            }
            Taken::Made => {
                if self.field.is_none()
                    && let Some(standby) = &self.standby
                {
                    let mut field = standby
                        .field
                        .sibling(device, self.center[0], self.center[1]);
                    for impulse in self.waiting.drain(..) {
                        field.add_impulse(impulse);
                    }
                    self.field = Some(field);
                }
                self.waiting.clear();
                self.version += 1;
            }
        }
        for field in self
            .field
            .iter_mut()
            .chain(self.standby.iter_mut().map(|s| &mut s.field))
        {
            field.update_timed(queue, encoder, self.dt, timer);
        }
        if let Some(collectors) = self.trails.take_repaint() {
            self.trail_painter
                .repaint(device, queue, encoder, &collectors);
        }
        // The foam field steps once the trails its new cells start from are painted.
        if let Some(centre) = self.sim_centre {
            let s = &self.trails.shading;
            let frame = crate::foam_sim::FoamFrame {
                dt: self.dt,
                centre,
                drift: self.trails.oil_drift,
                sterns: &self.sim_sterns,
                splashes: &self.trails.deposits,
                now: self.trails.time,
                trail_atlas: self.trail_painter.view(),
                tiles: &self.trails.rows,
                shading: [s.foam_start, s.foam_full, s.foam_cover, s.slick_full],
            };
            if self.foam_sim.update(device, queue, encoder, timer, &frame) {
                self.version += 1;
            }
        }
        if let Some(collectors) = self.trails.take_oil_repaint() {
            self.oil_painter
                .repaint(device, queue, encoder, &collectors);
        }
        if self.torpedoes.dirty {
            self.torpedoes.dirty = false;
            self.torpedo_painter
                .repaint(device, queue, encoder, &[&self.torpedoes.collector]);
        }
        queue.write_buffer(&self.uniforms, 0, bytemuck::cast_slice(&self.pack()));
        self.dt = 0.0;
    }

    /// The `Wake` uniform (`shaders/wake.wgsl`).
    fn pack(&self) -> Vec<[f32; 4]> {
        let mut data = vec![[0.0_f32; 4]; WAKE_VEC4S];
        if let Some(field) = &self.field {
            let sampling = field.sampling();
            data[0] = [
                sampling.origins[0][0],
                sampling.origins[0][1],
                sampling.origins[1][0],
                sampling.origins[1][1],
            ];
            data[1] = [
                sampling.weights[0],
                sampling.weights[1],
                sampling.world_size,
                0.0,
            ];
        } else {
            data[1] = [0.0, 0.0, WORLD_SIZE as f32, 0.0];
        }
        data[1][3] = self.bow_waves.mesh_growth as f32;
        let torpedo = self.torpedoes.center();
        data[2] = [
            self.trails.rows.len() as f32,
            self.trails.time as f32,
            self.torpedoes.time as f32,
            if self.torpedoes.painted() { 1.0 } else { 0.0 },
        ];
        data[3] = self.trails.reach.map(|v| v as f32);
        let mesh = self.bow_waves.mesh_centre;
        data[4] = [
            torpedo[0] as f32,
            torpedo[1] as f32,
            mesh[0] as f32,
            mesh[1] as f32,
        ];
        let s = &self.trails.shading;
        data[5] = [s.foam_start, s.foam_full, s.foam_cover, s.tear];
        data[6] = [s.bubble_start, s.bubble_full, s.bubbles, s.bubble_tear];
        data[7] = [s.lobes, s.lobe_scale, s.eddy_scale, s.eddy_fine];
        data[8] = [
            s.slick_full,
            s.calm,
            s.residue,
            f32::from(u8::from(self.bow_waves.mesh_fan)),
        ];
        let bow = &self.bow_waves;
        data[9] = [
            bow.slots.len() as f32,
            bow.time as f32,
            bow.pixel_angle as f32,
            bow.vertex_spacing as f32,
        ];
        data[10..14].copy_from_slice(&bow.tuning.knobs());
        let hull = &self.hull_foam;
        data[14] = [
            hull.count as f32,
            hull.time as f32,
            hull.pixel_angle as f32,
            hull.tuning.bow_length as f32,
        ];
        for (i, row) in self.trails.rows.iter().enumerate() {
            data[HEADER + i] = row.center;
            data[HEADER + ATLAS_CAPACITY + i] = row.slick_center;
            data[HEADER + 2 * ATLAS_CAPACITY + i] = row.foam_box;
            data[HEADER + 3 * ATLAS_CAPACITY + i] = row.slick_box;
            data[HEADER + 4 * ATLAS_CAPACITY + i] = row.box_;
        }
        let slots = crate::bow_waves::BOW_WAVE_SLOTS;
        for (i, slot) in bow.slots.iter().enumerate() {
            for (section, value) in slot.iter().enumerate() {
                data[HEADER + TILE_ROWS + section * slots + i] = *value;
            }
        }
        let hull_start = HEADER + TILE_ROWS + BOW_ROWS;
        data[hull_start..hull_start + hull.count * STRIDE]
            .copy_from_slice(&hull.values[..hull.count * STRIDE]);
        // The oil slicks (`shaders/oil.wgsl`): slots in use and the squares' edges, their reach, the surface's drift and
        // how far it has gone, then each slot's rows.
        let oil = &self.trails;
        let oil_start = hull_start + HULL_ROWS;
        data[oil_start] = [
            oil.oil_rows.len() as f32,
            OIL_NEAR_EXTENT as f32,
            OIL_FAR_EXTENT as f32,
            0.0,
        ];
        data[oil_start + 1] = oil.oil_reach.map(|v| v as f32);
        data[oil_start + 2] = [
            oil.oil_drift[0] as f32,
            oil.oil_drift[1] as f32,
            oil.oil_offset[0] as f32,
            oil.oil_offset[1] as f32,
        ];
        let rows = oil_start + OIL_HEADER;
        for (i, row) in oil.oil_rows.iter().enumerate() {
            data[rows + i] = row.near;
            data[rows + OIL_SLOTS + i] = row.far;
            data[rows + 2 * OIL_SLOTS + i] = row.near_box;
            data[rows + 3 * OIL_SLOTS + i] = row.far_box;
            data[rows + 4 * OIL_SLOTS + i] = row.box_;
        }
        // The splashes' deposits' rows, then the foam field's.
        let deposits = rows + OIL_ROWS;
        let field = deposits + crate::splash_foam::SPLASH_ROWS;
        let count = self
            .trails
            .deposits
            .rows(self.trails.time, &mut data[deposits..field]);
        data[field..field + 3].copy_from_slice(&self.foam_sim.rows(count));
        data
    }

    /// Clear every trail and the field (returning to port).
    pub fn reset(&mut self, device: &wgpu::Device) {
        self.trails.reset();
        self.torpedoes.reset();
        self.bow_waves.reset();
        self.hull_foam.reset();
        self.event_sequence = 0;
        self.foam_sim.reset();
        self.standby = None;
        self.anchor = None;
        self.taken = Taken::Kept;
        self.waiting.clear();
        if let Some(field) = &mut self.field {
            field.reset(device);
            self.version += 1;
        }
    }

    /// The wake fields: the focus's, then the one kept about the hull the view left.
    fn fields_mut(&mut self) -> impl Iterator<Item = &mut WakeField> {
        self.field
            .iter_mut()
            .chain(self.standby.iter_mut().map(|s| &mut s.field))
    }

    /// The narrowest footprint the fields carry ([`WakeField::least_radius`]); none without a field (Low).
    pub fn least_radius(&self) -> Option<f64> {
        self.field
            .as_ref()
            .or(self.standby.as_ref().map(|s| &s.field))
            .map(WakeField::least_radius)
    }

    /// Lay `impulse` on each field that covers it ([`WakeField::add_impulse`]). While the focus's field is yet to be
    /// made it waits for it, so what strikes the sea as the view reaches a hull is not lost.
    pub fn add_impulse(&mut self, impulse: WakeImpulse) {
        if self.taken == Taken::Made {
            self.waiting.push(impulse);
        }
        for field in self.fields_mut() {
            field.add_impulse(impulse);
        }
    }

    /// What `strike` at world `x`, `z` does to the sea: its waves ([`strike_impulse`]), laid as
    /// [`ShipWake::add_impulse`] on the fields there are, and a charge's white water, a deposit as its splash's
    /// ([`crate::splash_foam::SplashDeposits::lay_charge`]: the foam field lays it within its window, the surface draws
    /// it beyond and on Low). A round's and an aircraft's white water are their splashes' ([`WakeSplash`]).
    pub fn strike(&mut self, x: f64, z: f64, strike: &Strike) {
        if let Strike::Charge { kg, depth } = *strike {
            let now = self.trails.time;
            self.trails
                .deposits
                .lay_charge(now, x, z, charge_scale(kg, depth));
        }
        if let Some(impulse) = self
            .least_radius()
            .and_then(|least| strike_impulse(x, z, strike, least))
        {
            self.add_impulse(impulse);
        }
    }

    /// The hull the field kept beside the focus's follows, if there is one.
    pub fn standby_ship(&self) -> Option<&str> {
        self.standby.as_ref().map(|s| s.ship.as_str())
    }

    /// The field kept beside the focus's, if there is one.
    pub fn standby_field(&self) -> Option<&WakeField> {
        self.standby.as_ref().map(|s| &s.field)
    }

    /// The wake fields of a scene going away, moved `shift` into this one's metres (a theatre re-centre): the waves
    /// about the hulls run on instead of starting again from a calm sea. They are copied on `encoder` into this
    /// scene's own fields ([`WakeField::take_from`]); fields of another size (the graphics changed) are left.
    pub fn take_fields_from(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        old: &mut Self,
        shift: [f64; 2],
    ) {
        self.carry_fields(device, encoder, old, &|_| Some(shift));
    }

    /// [`ShipWake::take_fields_from`] into a scene laid on another chart (a theatre's re-lay): each field moved by
    /// where `relay` carries its centre, unturned, so only where the charts turn its water by less than
    /// [`RELAY_TURN`]; a field it would turn more starts calm.
    pub fn relay_fields_from(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        old: &mut Self,
        relay: &dyn naval_render::relay::SceneRelay,
    ) {
        use std::f64::consts::{PI, TAU};
        self.carry_fields(device, encoder, old, &|centre| {
            let to = relay.point2(centre);
            let turn = (relay.turn([centre[0], 0.0, centre[1]]) + PI).rem_euclid(TAU) - PI;
            (turn.abs() <= RELAY_TURN).then(|| [to[0] - centre[0], to[1] - centre[1]])
        });
    }

    /// The fields of `old` copied into this scene's, each moved by the shift `moved` gives at its centre (none: left).
    fn carry_fields(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        old: &mut Self,
        moved: &dyn Fn([f64; 2]) -> Option<[f64; 2]>,
    ) {
        // The foam field moves with the scene where the charts barely turn its water; elsewhere this scene's own starts
        // afresh, every cell cleared (the surface draws the foam itself while they come to hold it).
        if let Some(shift) = old.foam_sim.centre().and_then(moved)
            && self
                .foam_sim
                .take_from(device, &mut old.foam_sim, shift, self.trail_painter.view())
        {
            self.version += 1;
        }
        let Some(mine) = self.field.as_mut() else {
            return;
        };
        // Without one carried, this scene's own field, calm, serves the focus. One yet to be made takes what struck the
        // sea meanwhile.
        match old.field.take() {
            Some(mut theirs) => {
                if let Some(shift) = moved(theirs.center())
                    && mine.take_from(encoder, &mut theirs, shift)
                {
                    self.generators = std::mem::take(&mut old.generators);
                    self.taken = old.taken;
                }
            }
            None => {
                for mut impulse in old.waiting.drain(..) {
                    let Some(shift) = moved(impulse.from) else {
                        continue;
                    };
                    for point in [&mut impulse.from, &mut impulse.to] {
                        point[0] += shift[0];
                        point[1] += shift[1];
                    }
                    mine.add_impulse(impulse);
                }
            }
        }
        let standby = old.standby.take().and_then(|mut standby| {
            let shift = moved(standby.field.center())?;
            let mut field = self.field.as_ref()?.sibling(device, 0.0, 0.0);
            field
                .take_from(encoder, &mut standby.field, shift)
                .then_some(Standby {
                    field,
                    ship: standby.ship,
                    generators: standby.generators,
                })
        });
        self.standby = standby;
        self.anchor = old.anchor.take();
        self.version += 1;
    }

    /// Repaint the trail atlas from the current stamps whether or not they changed (for measurements).
    pub fn repaint_trails(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let collectors = self.trails.collectors();
        self.trail_painter.repaint(
            device,
            &mut naval_render::UploadSink::Queue(queue),
            encoder,
            &collectors,
        );
    }

    /// Stamps in the last repaint of the trail atlas and of the torpedo field, and wake field steps so far.
    pub fn diagnostics(&self) -> (u32, u32, u64) {
        (
            self.trail_painter.stamps(),
            self.torpedo_painter.stamps(),
            self.field.as_ref().map_or(0, |f| f.steps_run),
        )
    }

    /// The trail atlas texture, for inspection.
    pub fn trail_atlas(&self) -> &wgpu::Texture {
        self.trail_painter.texture()
    }

    /// The oil slicks: slots drawn, stamps in the last repaint of their atlas and patches on the water in all.
    pub fn oil_diagnostics(&self) -> (usize, u32, usize) {
        (
            self.trails.oil_rows.len(),
            self.oil_painter.stamps(),
            self.trails.slicks().map(|(_, slick)| slick.patches()).sum(),
        )
    }

    /// The oil atlas texture, for inspection.
    pub fn oil_atlas(&self) -> &wgpu::Texture {
        self.oil_painter.texture()
    }
}

/// The field's generators: its local swell from the nearest [`GENERATOR_HULLS`] hulls within [`GENERATOR_REACH`] of
/// its `centre`, those that have left it removed and those come into it added, each posed and pressed by her speed.
/// `generators` are its (hull id, bow, stern).
fn raise_waves(
    field: &mut WakeField,
    generators: &mut Vec<(String, u32, u32)>,
    centre: [f64; 2],
    ships: &[WakeShip<'_>],
) {
    let span =
        |ship: &WakeShip<'_>| (ship.position[0] - centre[0]).hypot(ship.position[2] - centre[1]);
    // The wake field takes 16 generators: its local swell goes to the nearest eight hulls.
    let mut nearby: Vec<&WakeShip<'_>> = ships
        .iter()
        .filter(|s| {
            (s.position[0] - centre[0]).abs() < GENERATOR_REACH
                && (s.position[2] - centre[1]).abs() < GENERATOR_REACH
        })
        .collect();
    nearby.sort_by(|a, b| span(a).total_cmp(&span(b)));
    nearby.truncate(GENERATOR_HULLS);
    let (keep, gone): (Vec<_>, Vec<_>) = generators
        .drain(..)
        .partition(|(id, ..)| nearby.iter().any(|s| s.id == id));
    for (_, bow, stern) in gone {
        field.generators.remove(bow);
        field.generators.remove(stern);
    }
    *generators = keep;
    let mut strength: f64 = 0.0;
    for ship in &nearby {
        if !generators.iter().any(|(id, ..)| id == ship.id) {
            let options = |depth: f64, radius: f64, offset_z: f64| GeneratorOptions {
                active: false,
                depth,
                radius,
                teleport_threshold: 100.0,
                offset: [ship.center_x, offset_z],
                breaks_to: [ship.center_x, ship.center_z],
                break_reach: ship.beam * BREAK_REACH,
            };
            let bow = field.generators.add(options(
                0.32,
                ship.beam * 0.28,
                ship.center_z - ship.length * 0.448,
            ));
            let stern = field.generators.add(options(
                0.18,
                ship.beam * 0.39,
                ship.center_z + ship.length * 0.448,
            ));
            if let (Some(bow), Some(stern)) = (bow, stern) {
                generators.push((ship.id.to_owned(), bow, stern));
            }
        }
        let Some(&(_, bow, stern)) = generators.iter().find(|(id, ..)| id == ship.id) else {
            continue;
        };
        let speed = ship.speed.abs() * surfaced(ship.position[1]);
        let ratio = (speed / ship.forward_speed.max(1e-3)).min(1.0);
        field.generators.update(bow, |g| {
            g.active = speed > 0.1;
            g.depth = 0.32 * ratio * ratio;
        });
        field.generators.update(stern, |g| {
            g.active = speed > 0.1;
            g.depth = 0.18 * ratio * ratio;
        });
        for id in [bow, stern] {
            field
                .generators
                .set_pose(id, ship.position[0], ship.position[2], ship.heading);
        }
        strength = strength.max(ratio);
    }
    field.foam_strength = 1.2 * strength;
}
