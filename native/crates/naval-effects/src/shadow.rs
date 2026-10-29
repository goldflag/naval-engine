//! The smoke's shadows (the `shadows` look: a trial until 2026-10-04, now the game's, drawn while the renderer's sun
//! shadow maps are): smoke shaded by the smoke between it and the sun, and its shadow on everything else.
//!
//! Each puff is relit alone, so a bank of gun smoke, a fire's column or a funnel's trail has no lit side and dark
//! side of its own. The layer's `prepare` draws an opacity shadow map along the light beside the occlusion map, and
//! the smoke materials read it per vertex (`shaders/shadow.wgsl`, `puff_light`): the sun's (or moon's) light on a puff
//! is what the other smoke between it and the light lets through.
//!
//! The map is a deep opacity map (Yuksel and Keyser 2008). Every puff of smoke, seen or not, is a ball to the light
//! ([`Blob`]: the gas pools, the fires' columns and flame bodies, the funnels' billows and trails). A small front map
//! ([`FRONT_SIZE`]², max blending) holds, per texel, where the first smoke begins along the light; a volume
//! ([`MAP_SIZE`]² × [`SLICES`], additive, eight slices a pass) holds the optical depth gathered by each of eight
//! depths behind that front, 2.5 m apart at the front and widening to about 100 m behind it. Measuring from each
//! texel's own front keeps the slices fine where the light is still strong, whatever the sun's height and however far
//! apart the smoke along one ray is, which slices across the whole depth of the map (or Fourier coefficients over
//! it) cannot. The trial's map was 512² × 16 slices, read per fragment; read per vertex, a quarter of the texels and
//! half the slices look the same at a fraction of the fill.
//!
//! The same balls go into the renderer's soft occluders' map (`naval_render::SunOccluders`, `SUN_OCCLUDER_SIZE`² over
//! the same square): per texel, where the smoke along the light begins and ends and all it stops. `naval::shadow`'s
//! `sun_shadow` and `water_sun_shadow` read it, so the ships, the sea, the aircraft and the land take the smoke's
//! shadow with no binding of their own.
//!
//! The map covers the smoke the camera sees ([`fit`]): a square across the light around every puff in view that is
//! more than a pixel or two across (where the camera draws it, the Earth's curvature lowering what is far off), at a
//! power-of-two texel size (0.5 m for a square up to 128 m, to 8 m for 2 km), its corner snapped to the front map's
//! texels so neither grid moves in the world as the camera or the smoke does. Where that square would pass a
//! kilometre, only the smoke larger on screen is fitted ([`PIXEL_TIERS`]), so smoke near the camera keeps fine
//! shadows; anything along the light over the square casts, in view or not (to [`CAST_REACH`] toward the light).
//! Smoke outside the square neither shades smoke nor casts, the shadows fading out over the square's rim.
//!
//! A puff shades itself already (the gas atlas, or its material's own shading), so it reads the map on the plane
//! touching its sun side: what lies before that plane is other smoke, and the sun side of a bank is lit as it was.
//! A plume or a puff alone is therefore unchanged. Flashes, firelight and burning gas are not shadowed.
//!
//! With the look off, or the sun's maps off, nothing is drawn and the materials' reads are skipped (the uniform's
//! `params.w` stays zero, and the renderer's soft occluders stay clear).

use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use glam::DVec3;
use naval_render::{
    Pipeline, PipelineId, RenderContext, SUN_OCCLUDER_SIZE, ShaderError, SunOccluderUniform,
    SunOccluders,
};

use crate::camera::CameraView;
use crate::combat::CombatEffects;
use crate::fire_batch::{FireBatch, FireMode};
use crate::gpu::{GrowBuffer, sampler_entry, storage_entry, texture_entry, uniform_entry};
use crate::math::{smooth, smoothstep};
use crate::particles::ParticlePool;

/// What the smoke materials import (`naval_effects::shadow`).
pub(crate) const SHADOW: naval_render::ShaderFile =
    naval_render::shader_file!("shaders/shadow.wgsl");
const CAST: naval_render::ShaderFile = naval_render::shader_file!("shaders/shadow_cast.wgsl");

/// The slices' width and height in texels (512 in the trial: a quarter of the fill, read per vertex since).
pub const MAP_SIZE: u32 = 256;
/// The front map's width and height in texels.
pub const FRONT_SIZE: u32 = 128;
/// Depths behind the front the volume keeps the gathered optical depth at (16 in the trial: one pass of eight now).
pub const SLICES: u32 = 8;
/// Passes of eight slices.
const PASSES: usize = (SLICES / 8) as usize;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// The soft occluders' first plane: where the smoke begins and ends along the light.
const SPAN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// Slice `i` lies `KNOT_STEP·i + KNOT_GROWTH·i²` metres behind the front at a texel of a metre or less (the two grow
/// with the texel beyond that): 0, 4.3, 12.2 … 106 m.
const KNOT_STEP: f64 = 2.5;
const KNOT_GROWTH: f64 = 1.8;
/// The finest and coarsest texels (m): a square of 128 m to 2 km.
const MIN_TEXEL: f64 = 0.5;
const MAX_TEXEL: f64 = 8.0;
/// How large on screen (its radius in pixels) a puff must be for the map to reach it: any puff more than a pixel or
/// two across while that keeps the square within [`FINE_SPAN`]; otherwise only the larger ones, so smoke near the
/// camera keeps fine shadows when more smoke kilometres away is in view too.
const PIXEL_TIERS: [f64; 3] = [1.5, 5.0, 16.0];
/// The widest square (m) taken for the sake of smoke that is small on screen: 4 m texels.
const FINE_SPAN: f64 = 1024.0;
/// How far toward the light of the smoke in view a puff still casts (m): the front map's half floats hold the depth
/// to about a thousandth, so the reach sets how finely a front is placed.
const CAST_REACH: f64 = 1500.0;
/// Optical depth through a gas puff's middle in the atlas's units (`body.b × GAS_DEPTH` there).
const GAS_CORE_DEPTH: f64 = 6.0;
/// The most a single puff stops (its opacity is never quite 1).
const MAX_OPACITY: f64 = 0.97;

/// A puff of smoke as the light sees it: a ball, thickest through its middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub centre: DVec3,
    pub radius: f64,
    /// Optical depth through the middle.
    pub optical: f64,
}

impl Blob {
    /// A ball of `radius` that stops `opacity` of the light through its middle; none for one too faint to matter.
    pub fn new(centre: DVec3, radius: f64, opacity: f64) -> Option<Self> {
        (opacity > 0.01 && radius > 0.05 && centre.is_finite()).then(|| Self {
            centre,
            radius,
            optical: -(1.0 - opacity.min(MAX_OPACITY)).ln(),
        })
    }
}

fn hidden_source(hidden: Option<&str>, source: &Option<crate::particles::SourceId>) -> bool {
    matches!((hidden, source), (Some(hidden), Some(source)) if &**source == hidden)
}

/// The share of a flipbook frame's half-width its gas fills as a ball: the bake frames each frame on its gas with a
/// margin (0.06 of the tile, `pack.py`), and the gas is ragged and thin at its rim, its dense body well inside.
const FLIPBOOK_GAS: f64 = 0.8;
/// Heat (the gas material's `heat_scale`) over which a puff's flame goes out: from the second, the fireball burns
/// through (`gas_sprite.wgsl`'s threshold `1 − heat × 0.38` passes all but its coolest gas), at the first only hot
/// pockets are left in smoke.
const GLOW_HEAT: [f64; 2] = [0.9, 1.9];
/// The same for a gun's propellant, whose heat is the share of its burning still to come (`propellant_flame` in
/// `gas_sprite.wgsl`): above the second it burns through, below the first only in pockets.
const PROPELLANT_GLOW_HEAT: [f64; 2] = [0.3, 0.9];

/// A gas pool's puffs (combat smoke, flak, aircraft smoke, soot), as `ParticlePool::publish_gas_into` and the gas
/// material make them opaque: the pool's fade, thinner as the puff disperses. With the flipbook look on (`sizes`, the
/// loaded clips), a puff playing a clip is its frame's gas, drawn smaller than the full-grown puff while the
/// simulation's gas is young (`place` in `gas_sprite.wgsl`), not the whole sprite. A fireball still burning casts
/// nothing: its own light on what is under it outweighs the sun it stops, and its shadow comes as it cools to smoke.
pub(crate) fn gas_blobs(
    pool: &ParticlePool,
    hidden: Option<&str>,
    sizes: Option<&crate::flipbook::ClipSizes>,
    out: &mut Vec<Blob>,
) {
    for p in pool.particles() {
        if p.age < 0.0 || p.age >= p.life || hidden_source(hidden, &p.source_id) {
            continue;
        }
        let t = p.age / p.life;
        let mut centre = p.position;
        let mut radius = p.current_size() / 2.0;
        if let Some(sizes) = sizes.filter(|_| p.flipbook > 0.0) {
            // The frame's gas, its foot kept where the full-grown puff's stands.
            let (scale, anchor) = sizes.frame(p.flipbook, t);
            centre.y -= radius * anchor * (1.0 - scale);
            radius *= scale * FLIPBOOK_GAS;
        }
        // Heat held past the puff's life is the simulation's to burn where it will (`flipbook_blasts::held_heat`):
        // only a recipe's own cooling puts a fireball out.
        let glow = if p.cooling < p.life {
            let heat = p.heat * (1.0 - smooth(p.age / p.cooling));
            let [cool, hot] = if p.propellant {
                PROPELLANT_GLOW_HEAT
            } else {
                GLOW_HEAT
            };
            smoothstep(heat, cool, hot)
        } else {
            0.0
        };
        // `EffectParticle::fade`.
        let fade = (1.0 - smooth((t - 0.18) / 0.82))
            * if p.fade_in > 0.0 {
                smooth(p.age / p.fade_in)
            } else {
                1.0
            };
        let dispersal = if p.dissipation_time > 0.0 {
            let d = (p.age - p.cooling).max(0.0) / p.dissipation_time;
            1.0 - (-d * d).exp()
        } else {
            0.0
        };
        // `puff_at` in `gas_sprite.wgsl`: optical depth falls as the puff thins, and its thin parts tear away.
        let thickness = p.density_now() / 3.0 * (1.0 - dispersal * 0.8);
        let solid = 1.0 - (-GAS_CORE_DEPTH * thickness).exp();
        let torn = 1.0
            - smoothstep(dispersal, 0.0, 0.3)
                * (1.0 - smoothstep(GAS_CORE_DEPTH, dispersal * 1.6, dispersal * 1.6 + 0.9));
        out.extend(Blob::new(
            centre,
            radius,
            p.opacity * fade * solid * torn * (1.0 - glow),
        ));
    }
}

/// A fire batch's smoke puffs or flame bodies (not its flames), as `FireBatch::publish_into` fades them.
pub(crate) fn fire_blobs(batch: &FireBatch, hidden: Option<&str>, out: &mut Vec<Blob>) {
    if batch.mode == FireMode::Flame {
        return;
    }
    for p in batch.particles() {
        if p.age < 0.0 || p.age >= p.life || hidden_source(hidden, &p.source_id) {
            continue;
        }
        let t = p.age / p.life;
        let extent = p.extent();
        let fade = (if p.fade_in > 0.0 {
            smooth(p.age / p.fade_in)
        } else {
            1.0
        }) * (1.0 - smooth((t - 0.45) / 0.55))
            * if p.thin > 0.0 {
                (p.size / extent).powf(p.thin).clamp(0.35, 1.0)
            } else {
                1.0
            };
        // A flame body thins into wisps late in its climb (`fireBodyInputs`).
        let wisps = if batch.mode == FireMode::Gas {
            1.0 - 0.7 * smoothstep(t, 0.6, 1.0)
        } else {
            1.0
        };
        let opacity = p.opacity * fade * wisps;
        if batch.plume && !p.billow {
            // A link spread across the wind in its pall (`fire_batch::pall_spread`) shades as much more of the sea
            // under it, and as much more thinly: a ball covering that much more, its optical depth spread over it.
            let wide = crate::fire_batch::pall_spread(p, extent).0;
            let thinned = 1.0 - (1.0 - opacity.clamp(0.0, MAX_OPACITY)).powf(1.0 / wide);
            out.extend(Blob::new(p.position, extent * wide.sqrt(), thinned));
            continue;
        }
        out.extend(Blob::new(p.position, extent, opacity));
    }
}

/// The funnels' billows: sprites of a cauliflower texture, densest in the middle.
pub(crate) fn billow_blobs(pool: &ParticlePool, hidden: Option<&str>, out: &mut Vec<Blob>) {
    for p in pool.particles() {
        if p.age < 0.0 || p.age >= p.life || hidden_source(hidden, &p.source_id) {
            continue;
        }
        let t = p.age / p.life;
        let fade = (1.0 - smooth((t - 0.18) / 0.82))
            * if p.fade_in > 0.0 {
                smooth(p.age / p.fade_in)
            } else {
                1.0
            };
        // The billow fades out toward the sea, as its material does.
        let above = smoothstep(p.position.y, 0.0, 4.0);
        out.extend(Blob::new(
            p.position,
            p.current_size() * 0.42,
            p.opacity * fade * above * 0.8,
        ));
    }
}

/// The map's place in the world for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// The map's axes in the world: across it, up it, and along the light (away from it).
    pub axes: [DVec3; 3],
    /// The map's corner along the first two axes, and the depth its depths count from (m).
    pub origin: DVec3,
    /// Metres per texel of the slices.
    pub texel: f64,
    /// Depth (m from the origin) just behind the furthest caster: the front map counts back from it.
    pub far: f64,
    /// The nearest depth toward the light a puff still casts from (m from the origin).
    pub near: f64,
}

impl Fit {
    /// The map's width (m).
    pub fn extent(&self) -> f64 {
        self.texel * f64::from(MAP_SIZE)
    }

    /// A world point in the map: metres across and up from its corner, and metres along the light from its origin.
    pub fn place(&self, point: DVec3) -> DVec3 {
        DVec3::new(
            point.dot(self.axes[0]),
            point.dot(self.axes[1]),
            point.dot(self.axes[2]),
        ) - self.origin
    }

    /// Whether a blob lies over the map's square and near enough the smoke in view to cast on it.
    fn casts(&self, blob: &Blob) -> bool {
        let at = self.place(blob.centre);
        let (r, extent) = (blob.radius, self.extent());
        at.x > -r
            && at.x < extent + r
            && at.y > -r
            && at.y < extent + r
            && at.z + r > self.near
            && at.z - r < self.far
    }
}

/// The light's frame: across (level), up the map, and along the light away from it. `light` points toward the light.
fn light_axes(light: DVec3) -> Option<[DVec3; 3]> {
    // A light under the horizon lights nothing.
    let toward = light.try_normalize().filter(|toward| toward.y > 0.0)?;
    let across = DVec3::Y.cross(toward);
    let across = if across.length_squared() < 1e-8 {
        DVec3::X
    } else {
        across.normalize()
    };
    Some([across, toward.cross(across), -toward])
}

/// Where the map goes for `camera`: a square across the light around the smoke in view (see the module's notes).
/// `previous` is the last frame's texel size, kept while the smoke still fits it and fills a good part of it, so the
/// map does not flick between two sizes. None with no smoke in view or no light.
pub fn fit(
    blobs: &[Blob],
    camera: &CameraView,
    light: DVec3,
    previous: Option<f64>,
) -> Option<Fit> {
    let axes = light_axes(light)?;
    let planes = camera.frustum();
    let (_, lens_y) = camera.lens();
    // A blob's radius on screen in pixels when it is in view: inside the four side planes and in front of the lens.
    let seen = |blob: &Blob| {
        let r = blob.radius;
        // Where the camera draws it: smoke far off is drawn lowered by the Earth's curvature.
        let drawn = camera.drawn(blob.centre);
        if planes[..5].iter().any(|plane| plane.distance(drawn) < -r) {
            return None;
        }
        let distance = (drawn - camera.position).length();
        Some(camera.height * 0.5 * lens_y * r / distance.max(r))
    };
    let across_light = |blob: &Blob| {
        DVec3::new(
            blob.centre.dot(axes[0]),
            blob.centre.dot(axes[1]),
            blob.centre.dot(axes[2]),
        )
    };
    // The smoke in view by how large it is on screen: bounds across the light, and where most of it is.
    #[derive(Clone, Copy)]
    struct Tier {
        low: DVec3,
        high: DVec3,
        weighted: DVec3,
        weight: f64,
    }
    impl Tier {
        fn span(&self) -> f64 {
            (self.high.x - self.low.x).max(self.high.y - self.low.y) * 1.04 + 4.0
        }
    }
    let mut tiers = [Tier {
        low: DVec3::splat(f64::INFINITY),
        high: DVec3::splat(f64::NEG_INFINITY),
        weighted: DVec3::ZERO,
        weight: 0.0,
    }; PIXEL_TIERS.len()];
    for blob in blobs {
        let Some(pixels) = seen(blob) else {
            continue;
        };
        let (at, r) = (across_light(blob), DVec3::splat(blob.radius));
        let w = pixels.min(300.0).powi(2);
        for (tier, least) in tiers.iter_mut().zip(PIXEL_TIERS) {
            if pixels >= least {
                tier.low = tier.low.min(at - r);
                tier.high = tier.high.max(at + r);
                tier.weighted += at * w;
                tier.weight += w;
            }
        }
    }
    // All of it where that keeps the texels fine; otherwise only the smoke that is larger on screen.
    let seen_tiers = tiers.iter().filter(|tier| tier.weight > 0.0);
    let (index, tier) = seen_tiers
        .clone()
        .enumerate()
        .find(|(_, tier)| tier.span() <= FINE_SPAN)
        .or_else(|| seen_tiers.enumerate().last())?;
    let least_pixels = PIXEL_TIERS[index];
    let (weighted, weight) = (tier.weighted, tier.weight);
    let (mut low, mut high) = (tier.low, tier.high);
    let span = tier.span();
    let needed = span / f64::from(MAP_SIZE);
    let mut texel = MIN_TEXEL;
    while texel < needed && texel < MAX_TEXEL {
        texel *= 2.0;
    }
    if let Some(previous) = previous
        && previous == texel * 2.0
        && needed > previous * 0.35
    {
        texel = previous;
    }
    let extent = texel * f64::from(MAP_SIZE);
    // Around all of it where it fits; otherwise around what fills the view most, kept inside the smoke's bounds.
    let middle = |axis: usize| {
        let (lo, hi) = (low[axis], high[axis]);
        if hi - lo <= extent {
            (lo + hi) * 0.5
        } else {
            (weighted[axis] / weight).clamp(lo + extent * 0.5, hi - extent * 0.5)
        }
    };
    // Snapped to the front map's texels (four of the slices'), so both grids stay where they are in the world.
    let step = extent / f64::from(FRONT_SIZE);
    let corner = |axis: usize| ((middle(axis) - extent * 0.5) / step).round() * step;
    let corner = DVec3::new(corner(0), corner(1), 0.0);
    if high.x - low.x > extent || high.y - low.y > extent {
        // The square does not hold all of it: the depths of what it does hold.
        (low.z, high.z) = (f64::INFINITY, f64::NEG_INFINITY);
        for blob in blobs {
            let at = across_light(blob) - corner;
            let r = blob.radius;
            if seen(blob).is_some_and(|pixels| pixels >= least_pixels)
                && at.x > -r
                && at.x < extent + r
                && at.y > -r
                && at.y < extent + r
            {
                low.z = low.z.min(at.z - r);
                high.z = high.z.max(at.z + r);
            }
        }
        if !low.z.is_finite() {
            return None;
        }
    }
    let depth_origin = ((low.z - CAST_REACH) / 64.0).floor() * 64.0;
    let mut fit = Fit {
        axes,
        origin: DVec3::new(corner.x, corner.y, depth_origin),
        texel,
        near: low.z - CAST_REACH - depth_origin,
        // Until the casters are known: everything in view.
        far: high.z - depth_origin,
    };
    // The front map counts back from just behind the furthest caster over the square.
    let mut far = f64::NEG_INFINITY;
    for blob in blobs {
        if fit.casts(blob) {
            far = far.max(fit.place(blob.centre).z + blob.radius);
        }
    }
    if !far.is_finite() {
        return None;
    }
    fit.far = far.min(fit.far) + 1.0;
    Some(fit)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlobInstance {
    place: [f32; 4],
    optical: [f32; 4],
}

/// `SmokeShadow` in `shadow.wgsl` and `shadow_cast.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct ShadowUniform {
    axis_u: [f32; 4],
    axis_v: [f32; 4],
    axis_d: [f32; 4],
    params: [f32; 4],
    cast: [f32; 4],
}

impl ShadowUniform {
    fn of(fit: &Fit) -> Self {
        let extent = fit.extent();
        let axis = |axis: DVec3, scale: f64, origin: f64| {
            [
                (axis.x * scale) as f32,
                (axis.y * scale) as f32,
                (axis.z * scale) as f32,
                (-origin * scale) as f32,
            ]
        };
        // The slices keep their fineness against the texel: no finer than the map can place a puff.
        let coarse = fit.texel.max(1.0);
        Self {
            axis_u: axis(fit.axes[0], 1.0 / extent, fit.origin.x),
            axis_v: axis(fit.axes[1], 1.0 / extent, fit.origin.y),
            axis_d: axis(fit.axes[2], 1.0, fit.origin.z),
            params: [
                fit.far as f32,
                (KNOT_STEP * coarse) as f32,
                (KNOT_GROWTH * coarse) as f32,
                1.0,
            ],
            cast: [
                extent as f32,
                (extent / f64::from(FRONT_SIZE)) as f32,
                1.0 / MAP_SIZE as f32,
                // How far depth along the light stands for depth under the sky: not at all with the light on the
                // horizon (its direct light is gone by then too, so the shadows leave without a step).
                (-fit.axes[2].y / 0.12).clamp(0.0, 1.0) as f32,
            ],
        }
    }
}

/// What the last frame's map held, for cost notes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadowStats {
    /// Balls drawn into the map (once into the front map, twice into the slices).
    pub casters: u32,
    /// Metres per texel of the slices (0 with no map drawn).
    pub texel: f32,
    /// Depth of the map along the light (m).
    pub depth: f32,
}

struct Maps {
    front: wgpu::TextureView,
    slices: wgpu::TextureView,
    /// Permanent 2D attachment views: wgpu 30's DX12 backend retains temporary 3D-slice RTVs until its pooled
    /// command encoder is destroyed, accumulating eight descriptors per frame even after the GPU finishes.
    slice_targets: [wgpu::TextureView; SLICES as usize],
    /// The renderer's soft occluders' two planes (`naval_render::SunOccluders`), drawn here and copied over: where the
    /// smoke along each texel begins and ends (max blending), and its optical depth (added).
    span: wgpu::Texture,
    optical: wgpu::Texture,
    /// The front map and each slices pass's first slice (group 1 of the slices passes).
    passes: [wgpu::BindGroup; PASSES],
}

/// The opacity shadow map and its passes.
pub(crate) struct SmokeShadows {
    front_pipeline: PipelineId,
    slices_pipeline: PipelineId,
    /// The smoke into the renderer's soft occluders' map, which the ships, the sea and every other receiver of the
    /// sun's shadows read.
    occluder_pipeline: PipelineId,
    cast_layout: wgpu::BindGroupLayout,
    pass_layout: wgpu::BindGroupLayout,
    /// `SmokeShadow`: the casters read it and the smoke materials do (scene reads, binding 40).
    pub uniform: wgpu::Buffer,
    pub sampler: wgpu::Sampler,
    blobs: GrowBuffer,
    cast_group: Option<(u64, wgpu::BindGroup)>,
    /// The maps, made when the trial is first on; `placeholder` stands in for them in the scene reads until then.
    maps: Option<Maps>,
    placeholder: (wgpu::TextureView, wgpu::TextureView),
    /// Working room kept between frames.
    scratch: Vec<Blob>,
    instances: Vec<BlobInstance>,
    texel: Option<f64>,
    live: bool,
    pub stats: ShadowStats,
    /// With `NAVAL_EFFECTS_SHADOW_LOG` set, the stats are printed once a hundred frames (for cost notes).
    log: Option<u64>,
}

/// The scene-reads layout entries of `shadow.wgsl` (bindings 40 to 43): read per vertex (`puff_light`), so the
/// fragment stage's sampled textures stay as they were without the map.
pub(crate) fn reads_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
    let vertex = wgpu::ShaderStages::VERTEX;
    [
        uniform_entry(40, vertex),
        texture_entry(41, vertex, wgpu::TextureViewDimension::D2, true),
        texture_entry(42, vertex, wgpu::TextureViewDimension::D2Array, true),
        sampler_entry(43, vertex),
    ]
}

fn map_texture(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32, u32),
    dimension: wgpu::TextureDimension,
    attachment: bool,
) -> wgpu::Texture {
    let attachment = if attachment {
        wgpu::TextureUsages::RENDER_ATTACHMENT
    } else {
        wgpu::TextureUsages::empty()
    };
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: size.2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | attachment,
        view_formats: &[],
    })
}

/// A map (or one layer of the slices array) as a pass's target, cleared first.
fn cleared(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    }
}

impl SmokeShadows {
    /// Registers the shaders (before the materials that import `naval_effects::shadow`) and builds the casters'
    /// pipelines. The maps themselves wait for the trial.
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        ctx.add_shader(SHADOW)?;
        ctx.add_shader(CAST)?;
        let device = ctx.device.clone();
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let cast_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke shadow casters"),
            entries: &[
                uniform_entry(0, vf),
                storage_entry(1, wgpu::ShaderStages::VERTEX),
            ],
        });
        let pass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke shadow slices pass"),
            entries: &[
                texture_entry(0, fragment, wgpu::TextureViewDimension::D2, true),
                sampler_entry(1, fragment),
                uniform_entry(2, fragment),
            ],
        });
        let blend = |operation: wgpu::BlendOperation| {
            let component = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation,
            };
            Some(wgpu::BlendState {
                color: component,
                alpha: component,
            })
        };
        let recipe = |layouts: Vec<wgpu::BindGroupLayout>,
                      vertex: &'static str,
                      fragment: &'static str,
                      targets: usize,
                      operation: wgpu::BlendOperation|
         -> naval_render::PipelineRecipe {
            Box::new(move |build| {
                let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                    layouts.iter().map(Some).collect();
                let layout = build
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(build.label),
                        bind_group_layouts: &groups,
                        immediate_size: 0,
                    });
                let targets = vec![
                    Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: blend(operation),
                        write_mask: wgpu::ColorWrites::ALL,
                    });
                    targets
                ];
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some(vertex),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some(fragment),
                            compilation_options: Default::default(),
                            targets: &targets,
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            })
        };
        let front_pipeline = ctx.add_pipeline(
            "smoke shadow front",
            CAST.path,
            Vec::new(),
            recipe(
                vec![cast_layout.clone()],
                "front_vertex",
                "front",
                1,
                wgpu::BlendOperation::Max,
            ),
        )?;
        let slices_pipeline = ctx.add_pipeline(
            "smoke shadow slices",
            CAST.path,
            Vec::new(),
            recipe(
                vec![cast_layout.clone(), pass_layout.clone()],
                "slices_vertex",
                "slices",
                8,
                wgpu::BlendOperation::Add,
            ),
        )?;
        let occluder_layouts = [cast_layout.clone()];
        let occluder_pipeline = ctx.add_pipeline(
            "smoke sun occluders",
            CAST.path,
            Vec::new(),
            Box::new(move |build| {
                let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                    occluder_layouts.iter().map(Some).collect();
                let layout = build
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(build.label),
                        bind_group_layouts: &groups,
                        immediate_size: 0,
                    });
                let target = |format, operation| {
                    let component = wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation,
                    };
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: component,
                            alpha: component,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                };
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some("slices_vertex"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some("occluders"),
                            compilation_options: Default::default(),
                            targets: &[
                                target(SPAN_FORMAT, wgpu::BlendOperation::Max),
                                target(FORMAT, wgpu::BlendOperation::Add),
                            ],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            }),
        )?;
        // Zeroed: `params.w` 0, the trial off.
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke shadow"),
            size: std::mem::size_of::<ShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = crate::gpu::sampler(
            &device,
            "smoke shadow",
            wgpu::AddressMode::ClampToEdge,
            false,
            1,
        );
        let placeholder = (
            map_texture(
                &device,
                "smoke shadow front placeholder",
                (1, 1, 1),
                wgpu::TextureDimension::D2,
                false,
            )
            .create_view(&wgpu::TextureViewDescriptor::default()),
            map_texture(
                &device,
                "smoke shadow slices placeholder",
                (1, 1, 1),
                wgpu::TextureDimension::D2,
                false,
            )
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
        );
        Ok(Self {
            front_pipeline,
            slices_pipeline,
            occluder_pipeline,
            cast_layout,
            pass_layout,
            uniform,
            sampler,
            blobs: GrowBuffer::new(
                &device,
                "smoke shadow casters",
                wgpu::BufferUsages::STORAGE,
                4096,
            ),
            cast_group: None,
            maps: None,
            placeholder,
            scratch: Vec::new(),
            instances: Vec::new(),
            texel: None,
            live: false,
            stats: ShadowStats::default(),
            log: std::env::var_os("NAVAL_EFFECTS_SHADOW_LOG").map(|_| 0),
        })
    }

    /// The front map and the slices the scene reads bind (bindings 41 and 42): the maps, or their stand-ins.
    pub fn views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        match &self.maps {
            Some(maps) => (&maps.front, &maps.slices),
            None => (&self.placeholder.0, &self.placeholder.1),
        }
    }

    fn make_maps(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Maps {
        let front = map_texture(
            device,
            "smoke shadow front",
            (FRONT_SIZE, FRONT_SIZE, 1),
            wgpu::TextureDimension::D2,
            true,
        )
        .create_view(&wgpu::TextureViewDescriptor::default());
        let slices_texture = map_texture(
            device,
            "smoke shadow slices",
            (MAP_SIZE, MAP_SIZE, SLICES),
            wgpu::TextureDimension::D2,
            true,
        );
        let slices = slices_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let slice_targets = std::array::from_fn(|slice| {
            slices_texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: slice as u32,
                array_layer_count: Some(1),
                ..Default::default()
            })
        });
        let passes: [_; PASSES] = std::array::from_fn(|pass| {
            let first = (pass * 8) as f32;
            let first_slice = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("smoke shadow first slice"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&first_slice, 0, bytes_of(&[first, 0.0, 0.0, 0.0]));
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("smoke shadow slices pass"),
                layout: &self.pass_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&front),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: first_slice.as_entire_binding(),
                    },
                ],
            })
        });
        let occluder_target = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: SUN_OCCLUDER_SIZE,
                    height: SUN_OCCLUDER_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        Maps {
            front,
            slices,
            slice_targets,
            passes,
            span: occluder_target("smoke sun occluder span", SPAN_FORMAT),
            optical: occluder_target("smoke sun occluder optical depth", FORMAT),
        }
    }

    /// Draws the frame's map when the trial is on (nothing otherwise). True when the maps were just made: the scene
    /// reads must be rebuilt over them.
    pub fn prepare(
        &mut self,
        ctx: &mut naval_render::PrepareContext<'_>,
        effects: &CombatEffects,
        camera: &CameraView,
    ) -> bool {
        let (device, queue, pipelines, timer) = (ctx.device, ctx.queue, ctx.pipelines, ctx.timer);
        let occluders = ctx.sun_occluders;
        let encoder = &mut *ctx.encoder;
        // Through the frame's staging belt, as the renderer's own zeroing of the soft occluders is (this write must
        // land after it).
        let uploads = &mut *ctx.uploads;
        // On with the look and the sun's shadow maps (the Graphics menu's shadows): the map is the smoke's shadow on
        // everything else too.
        if !effects.trial.shadows || !ctx.shadow_maps.iter().any(|map| map.drawn) {
            if self.live {
                // Switched off: the materials stop reading the map.
                uploads.write_buffer(&self.uniform, 0, bytes_of(&ShadowUniform::default()));
                self.live = false;
                self.stats = ShadowStats::default();
            }
            return false;
        }
        let made = self.maps.is_none();
        if made {
            self.maps = Some(self.make_maps(device, queue));
        }
        let mut blobs = std::mem::take(&mut self.scratch);
        blobs.clear();
        effects.shadow_blobs(&mut blobs);
        let fit = fit(&blobs, camera, effects.lighting.sun_direction, self.texel);
        self.texel = fit.map(|fit| fit.texel);
        self.instances.clear();
        if let Some(fit) = &fit {
            self.instances
                .extend(blobs.iter().filter(|blob| fit.casts(blob)).map(|blob| {
                    let at = fit.place(blob.centre);
                    BlobInstance {
                        place: [at.x as f32, at.y as f32, at.z as f32, blob.radius as f32],
                        optical: [blob.optical as f32, 0.0, 0.0, 0.0],
                    }
                }));
        }
        self.scratch = blobs;
        let uniform = match &fit {
            Some(fit) if !self.instances.is_empty() => ShadowUniform::of(fit),
            _ => ShadowUniform::default(),
        };
        uploads.write_buffer(&self.uniform, 0, bytes_of(&uniform));
        self.live = uniform.params[3] > 0.0;
        self.stats = match &fit {
            Some(fit) if self.live => ShadowStats {
                casters: self.instances.len() as u32,
                texel: fit.texel as f32,
                depth: (fit.far - fit.near) as f32,
            },
            _ => ShadowStats::default(),
        };
        if let Some(frames) = &mut self.log {
            *frames += 1;
            if *frames % 100 == 0 {
                eprintln!(
                    "[effects] smoke shadows: {} of {} puffs cast, {} m texels, {:.0} m along the light",
                    self.stats.casters,
                    self.scratch.len(),
                    self.stats.texel,
                    self.stats.depth
                );
            }
        }
        if !self.live {
            return made;
        }
        self.blobs.write_to(
            device,
            &mut naval_render::UploadSink::Belt(uploads),
            cast_slice(&self.instances),
        );
        if self
            .cast_group
            .as_ref()
            .is_none_or(|(version, _)| *version != self.blobs.version)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("smoke shadow casters"),
                layout: &self.cast_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.blobs.buffer.as_entire_binding(),
                    },
                ],
            });
            self.cast_group = Some((self.blobs.version, group));
        }
        let cast_group = &self.cast_group.as_ref().expect("built above").1;
        let maps = self.maps.as_ref().expect("made above");
        let count = self.instances.len() as u32;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smoke shadow front"),
                color_attachments: &[Some(cleared(&maps.front))],
                timestamp_writes: timer.render_pass("smoke shadow front"),
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(self.front_pipeline));
            pass.set_bind_group(0, cast_group, &[]);
            pass.draw(0..6, 0..count);
        }
        for (index, group) in maps.passes.iter().enumerate() {
            let attachments: [_; 8] =
                std::array::from_fn(|slice| Some(cleared(&maps.slice_targets[index * 8 + slice])));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smoke shadow slices"),
                color_attachments: &attachments,
                timestamp_writes: timer
                    .render_pass(["smoke shadow slices 0-7", "smoke shadow slices 8-15"][index]),
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(self.slices_pipeline));
            pass.set_bind_group(0, cast_group, &[]);
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..6, 0..count);
        }
        // The same balls into the renderer's soft occluders: where the smoke along each texel begins and ends, and its
        // whole optical depth, copied into the buffer every receiver of the sun's shadows reads.
        {
            let views = [&maps.span, &maps.optical]
                .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smoke sun occluders"),
                color_attachments: &[Some(cleared(&views[0])), Some(cleared(&views[1]))],
                timestamp_writes: timer.render_pass("smoke sun occluders"),
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(self.occluder_pipeline));
            pass.set_bind_group(0, cast_group, &[]);
            pass.draw(0..6, 0..count);
        }
        for (texture, offset, texel_bytes) in [
            (&maps.span, 0, 4),
            (&maps.optical, SunOccluders::SPAN_BYTES, 2),
        ] {
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &occluders.texels,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(SUN_OCCLUDER_SIZE * texel_bytes),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: SUN_OCCLUDER_SIZE,
                    height: SUN_OCCLUDER_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
        uploads.write_buffer(
            &occluders.uniforms,
            0,
            bytes_of(&SunOccluderUniform {
                axis_u: uniform.axis_u,
                axis_v: uniform.axis_v,
                axis_d: uniform.axis_d,
                // The span plane's fronts count back from the far depth; its backs count on from the depth origin.
                params: [uniform.params[0], 0.0, 0.0, 1.0],
            }),
        );
        made
    }
}

const _: () = assert!(
    SLICES.is_multiple_of(8) && PASSES <= 2,
    "passes of eight slices"
);

#[cfg(test)]
mod tests {
    use super::*;
    use naval_render::{Camera, FrameTime};

    fn view() -> CameraView {
        let camera = Camera::look_at(
            glam::Vec3::new(0.0, 30.0, 300.0),
            glam::Vec3::new(0.0, 30.0, 0.0),
        );
        CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()))
    }

    fn blob(x: f64, y: f64, z: f64, radius: f64) -> Blob {
        Blob::new(DVec3::new(x, y, z), radius, 0.9).unwrap()
    }

    #[test]
    fn the_map_fits_the_smoke_in_view_and_snaps_to_its_texels() {
        let light = DVec3::new(-0.4, 0.8, 0.2);
        let blobs = [blob(0.0, 20.0, 0.0, 10.0), blob(60.0, 40.0, -30.0, 15.0)];
        let fit = fit(&blobs, &view(), light, None).expect("smoke in view");
        assert_eq!(fit.texel, 0.5, "a bank under 128 m takes the finest texel");
        for blob in &blobs {
            let at = fit.place(blob.centre);
            assert!(at.x > blob.radius && at.x < fit.extent() - blob.radius);
            assert!(at.y > blob.radius && at.y < fit.extent() - blob.radius);
            assert!(at.z - blob.radius >= fit.near && at.z + blob.radius <= fit.far);
            assert!(fit.casts(blob));
        }
        assert_eq!((fit.origin.x / fit.texel).fract(), 0.0);
        assert_eq!((fit.origin.y / fit.texel).fract(), 0.0);
        // The smoke drifts less than a texel: the map stays where it is.
        let moved = blobs.map(|b| Blob {
            centre: b.centre + fit.axes[0] * 0.1,
            ..b
        });
        let again = super::fit(&moved, &view(), light, Some(fit.texel)).unwrap();
        assert_eq!(
            (again.origin.x, again.origin.y),
            (fit.origin.x, fit.origin.y)
        );
    }

    #[test]
    fn smoke_out_of_view_casts_but_does_not_move_the_map() {
        let light = DVec3::new(0.0, 1.0, 0.0);
        let seen = [blob(0.0, 20.0, 0.0, 10.0)];
        let alone = fit(&seen, &view(), light, None).unwrap();
        // Straight above the first, far over the top of the view, and another behind the camera.
        let with = [
            seen[0],
            blob(0.0, 600.0, 0.0, 10.0),
            blob(0.0, 20.0, 900.0, 10.0),
        ];
        let fitted = fit(&with, &view(), light, None).unwrap();
        assert_eq!(
            (fitted.origin.x, fitted.origin.y),
            (alone.origin.x, alone.origin.y)
        );
        assert!(fitted.casts(&with[1]), "over the square, toward the light");
        assert!(!fitted.casts(&with[2]), "off the square");
        assert!(
            fit(&with[2..], &view(), light, None).is_none(),
            "nothing in view"
        );
    }

    #[test]
    fn smoke_far_off_and_small_on_screen_does_not_coarsen_the_map() {
        let light = DVec3::new(0.0, 1.0, 0.0);
        // A bank close by, and a puff 3 km down the view, a few pixels across.
        let near = [blob(0.0, 20.0, 0.0, 10.0), blob(40.0, 25.0, -20.0, 12.0)];
        let far = blob(100.0, 30.0, -3000.0, 12.0);
        let fitted = fit(&[near[0], near[1], far], &view(), light, None).unwrap();
        assert_eq!(fitted.texel, 0.5);
        assert!(fitted.casts(&near[0]) && fitted.casts(&near[1]) && !fitted.casts(&far));
        // Alone, the far puff has the map.
        assert!(fit(&[far], &view(), light, None).unwrap().casts(&far));
    }

    #[test]
    fn a_puff_casts_the_gas_it_draws_and_a_burning_fireball_none() {
        use crate::flipbook::{ClipSizes, clip};
        use crate::particles::{ParticlePool, PoolKind};
        // A 20 m puff a tenth through its life, playing a blast's clip; and the same puff with its heat.
        let pool = |heat: f64, cooling: f64, age: f64| {
            let mut pool = ParticlePool::new(4, PoolKind::Gas);
            let puff = pool.emit(DVec3::new(0.0, 30.0, 0.0), None);
            puff.size = 20.0;
            puff.life = 10.0;
            puff.age = age;
            puff.opacity = 0.8;
            puff.flipbook = clip::BLAST;
            puff.heat = heat;
            puff.cooling = cooling;
            pool
        };
        let cast = |pool: &ParticlePool, sizes: Option<&ClipSizes>| {
            let mut out = Vec::new();
            gas_blobs(pool, None, sizes, &mut out);
            out
        };
        // Without the flipbook look, the whole puff.
        let smoke = pool(0.0, 1.0, 1.0);
        let whole = cast(&smoke, None);
        assert_eq!(whole.len(), 1);
        assert_eq!(
            (whole[0].centre, whole[0].radius),
            (DVec3::new(0.0, 30.0, 0.0), 10.0)
        );
        // Its clip draws the gas at half the grown puff's size, on a foot 0.9 half tiles under its centre: the ball is
        // that gas, its foot where the grown puff's stands.
        let sizes = ClipSizes::even(0.5, 0.9);
        let drawn = cast(&smoke, Some(&sizes));
        assert!((drawn[0].radius - 10.0 * 0.5 * FLIPBOOK_GAS).abs() < 1e-6);
        assert!((drawn[0].centre.y - (30.0 - 10.0 * 0.9 * 0.5)).abs() < 1e-6);
        assert_eq!(drawn[0].optical, whole[0].optical);
        // Burning through (heat 3 cooling over 4 s), a fireball casts nothing; cooled, it casts as smoke; heat held
        // past its life (the simulation's to burn) casts as smoke all along.
        assert!(cast(&pool(3.0, 4.0, 1.0), Some(&sizes)).is_empty());
        assert_eq!(cast(&pool(3.0, 4.0, 3.9), Some(&sizes)).len(), 1);
        assert_eq!(cast(&pool(3.0, 60.0, 1.0), Some(&sizes)).len(), 1);
    }

    #[test]
    fn a_wide_view_takes_a_coarser_texel_and_keeps_it() {
        let light = DVec3::new(0.3, 0.9, 0.1);
        let blobs = [blob(-150.0, 20.0, 0.0, 10.0), blob(150.0, 20.0, 0.0, 10.0)];
        let wide = fit(&blobs, &view(), light, None).unwrap();
        assert_eq!(wide.texel, 2.0);
        // Narrower: it would fit the finer texel, but not by much, so the map keeps its size.
        let closer = [blob(-95.0, 20.0, 0.0, 10.0), blob(95.0, 20.0, 0.0, 10.0)];
        assert_eq!(fit(&closer, &view(), light, Some(2.0)).unwrap().texel, 2.0);
        assert_eq!(fit(&closer, &view(), light, None).unwrap().texel, 1.0);
    }
}
