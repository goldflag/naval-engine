//! Photo mode's optics (`native/docs/photo.md`, "The lens"): the flare a camera's lens draws from the frame's bright
//! sources and the sun, and the lens's character the display's finish gives the frame (its chromatic aberration and
//! distortion, `shaders/display_pass.wgsl`) and the depth of field's bokeh (its aperture's blades).
//!
//! The flare, in the scene's radiance before the tone map (so an HDR output shows it above white):
//! - **Sources**: the frame's radiance above twice its white (the eye's exposure and photo mode's EV), gathered in
//!   64-pixel tiles (each tile's flux, its centroid and the area it covers), and the sun. The sun's disc the sky draws
//!   is dimmed for the eye (#380) and wider than the sun, so the lens takes the sun's own light, the irradiance the
//!   world is lit by ([`FlareSun::irradiance`]), over the solid angle of a pixel: its flux in the frame's units. What
//!   of it reaches the lens is measured where its disc is drawn in the frame (the disc's radiance over the sky about it
//!   against the radiance it has unobstructed: a hull or a cloud in front dims it); a sun outside the frame is taken
//!   as clear. A sun beyond the lens's field fades out ([`acceptance`]).
//! - **Ghosts** ([`GHOSTS`]): images of the aperture formed by light reflected twice between the lens's surfaces, on
//!   the line from the source through the frame's centre at [`Ghost::along`] times the source's offset, a size of
//!   [`Ghost::size`] frame heights at f/4 (an image of the aperture: smaller and brighter stopped down), each carrying
//!   the source's flux times the two surfaces' reflectances, spread over its area. A coated surface reflects a few
//!   tenths of a percent, tinted by its coating's thin film ([`coated_reflectance`]); an uncoated one about 4–6 %
//!   (Fresnel, [`uncoated_reflectance`]).
//! - **The starburst**: diffraction at the aperture's straight blade edges. Each edge sends a spike along its normal,
//!   both ways: an even count of blades gives that many spikes (opposite edges share a line), an odd count twice as
//!   many. Along a spike the flux per unit length falls as `λ N / r²` ([`spike_coefficient`]), so a stopped-down lens
//!   draws longer spikes, red reaches further than blue, and a brighter source longer ones. A round aperture draws none.
//! - **Veiling glare**: light scattered across the frame by the lens's surfaces and barrel, a share `η` of what comes
//!   in spread by a heavy-tailed kernel (about `r^-2.5`, a sum of Gaussians an octave apart). The coated lens scatters
//!   [`Coating::Coated`]'s 1 %, of the sun only: the frame's own glow is the game's look already. The uncoated lens
//!   scatters 6 % of everything, which lifts the shadows near bright light and lowers the contrast of the whole frame.
//!
//! None of it draws where no source passes: an ordinary frame comes out of the composite as it went in.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::{compute_recipe, fullscreen_recipe};
use crate::shaders::{ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;

pub const PHOTO_OPTICS_SHADER: ShaderFile = crate::shader_file!("shaders/post/photo_optics.wgsl");

/// The flare's targets' format (the scene's).
const FORMAT: wgpu::TextureFormat = crate::HDR_FORMAT;
/// Pixels a side of the sources' tiles.
pub const TILE: u32 = 64;
/// The veiling glare's pyramid: a quarter-resolution base halved until it is two pixels or so across, at most this
/// many levels ([`glare_levels`]).
pub const MAX_GLARE_LEVELS: usize = 12;
/// Each glare level's share against the next finer: `2^-0.5`, an `r^-2.5` tail's energy per octave.
pub const GLARE_FALLOFF: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// A pyramid level's spread (a Gaussian's sigma, full-resolution pixels) at the base: the base's 4 × 4 average and
/// the up-chain's tents; each level doubles it.
pub const GLARE_SIGMA0: f32 = 4.8;
/// Ghosts and spikes per source, and the quad's six corners each.
pub const SPRITES: u32 = 8 + 10;
/// The frame's radiance counts as a source above this many times its white (graded), in full by twice it.
pub const SOURCE_THRESHOLD: f32 = 2.0;
/// A ghost or spike fainter than this (graded) at its brightest is not drawn.
pub const CUTOFF: f32 = 2e-4;
/// The flare's targets hold half floats (65504 at most): the composite stops at this radiance and each ghost or spike
/// at [`SPRITE_CEILING`], so what they add stays finite where they pile up (all far past white: the display clips them
/// the same).
pub const COMPOSITE_CEILING: f32 = 16384.0;
pub const SPRITE_CEILING: f32 = 1024.0;
/// The optics' targets' versions start here, apart from the frame's resources' and the depth of field's: the display
/// caches its bind group by its source's version, so two textures must never share one.
const FIRST_VERSION: u64 = 1 << 48;
/// The wavelengths (mm) the red, green and blue channels stand for.
pub const WAVELENGTHS_MM: [f32; 3] = [610e-6, 550e-6, 465e-6];
/// The f-number the ghosts' sizes are given at.
const GHOST_APERTURE: f32 = 4.0;
/// The point lights the flare takes: the frame's eight brightest at the camera (the list is brightest first:
/// [`crate::lights::PointLights::brightest`]), sources 1 to 8 (the sun is 0, the tiles follow).
pub const MAX_LIGHTS: usize = 8;
/// A point light's image is taken as this wide (m): a flash's or a fire's core.
pub const LIGHT_SIZE_M: f32 = 2.0;
/// A point light's image is looked for in a window up a column this tall above it (m), from a sixth of it under the
/// light: a fire's flames rise above the light that stands for them (a few metres over the burning compartment, inside
/// the hull from most sides). The lens takes the light where the frame shows its image (the flame or the flash, at
/// least as bright as the frame's white), none where nothing bright shows (hidden behind a hull).
pub const LIGHT_COLUMN_M: f32 = 6.0;
/// The lens's widest aperture: the iris is open (round) there and has closed to its blades' polygon two stops down.
pub const WIDEST: f32 = 1.4;

/// A lens's coating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Coating {
    /// A modern multi-coated lens: a few tenths of a percent reflected per surface.
    #[default]
    Coated,
    /// A 1940s lens without coatings: every glass surface reflects 4–6 %.
    Uncoated,
}

impl Coating {
    /// The shares of the sun's light the lens scatters across the frame: into a halo about it (falling as `r^-2.5`)
    /// and a plateau over the whole image circle (off the barrel and the mounts). 1 % in all for a coated lens, 6 %
    /// for an uncoated one.
    pub fn sun_glare(self) -> (f32, f32) {
        match self {
            Coating::Coated => (0.007, 0.003),
            Coating::Uncoated => (0.03, 0.03),
        }
    }

    /// The shares of the frame's own light the lens scatters (halo, plateau): none for a coated lens, whose glow the
    /// game's look has already; the uncoated lens's 6 %, which lowers the whole frame's contrast.
    pub fn frame_glare(self) -> (f32, f32) {
        match self {
            Coating::Coated => (0.0, 0.0),
            Coating::Uncoated => (0.03, 0.03),
        }
    }

    /// Whether the glare is of the whole frame (the uncoated lens), not only the sun's.
    pub fn glares_everything(self) -> bool {
        let (halo, plateau) = self.frame_glare();
        halo + plateau > 0.0
    }
}

/// The sun (or the moon) as the lens's flare takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlareSun {
    /// Unit world vector toward it.
    pub direction: Vec3,
    /// Its irradiance at the camera (linear RGB, the scene's radiance units times a steradian): the sun's own light
    /// through the air, the light the world is lit by less the readability floor that holds it up as it sets.
    pub irradiance: Vec3,
    /// The body's own angular radius (radians): its image's size for the starburst.
    pub angular_radius: f32,
    /// The angular radius of the disc the sky draws for it (radians): the frame's sources leave it out, and its
    /// radiance is measured inside it.
    pub drawn_radius: f32,
    /// The radiance the drawn disc has unobstructed, averaged over it (linear RGB): what the measured disc is set
    /// against.
    pub disc_radiance: Vec3,
}

/// A camera lens's character in photo mode ([`crate::photo::PhotoPost::optics`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Optics {
    pub coating: Coating,
    /// The aperture's blades: 0 round, else the count (5, 6, 8).
    pub blades: u32,
    /// The f-number the aperture is set to (the depth of field's too).
    pub aperture: f32,
    /// The focal length (mm) on a full-frame camera.
    pub focal_mm: f32,
    /// Lateral chromatic aberration, 0–1 ([`chromatic_shift`]).
    pub chromatic: f32,
    /// The lens's distortion, following the focal length ([`distortion_of`]).
    pub distortion: bool,
    /// The sun (or moon) the flare takes, when it is up.
    pub sun: Option<FlareSun>,
    /// The scene's point lights the flare takes (the flashes, fires and lightning): their light by the inverse square
    /// (their cut-off is the shading's budget, not the light's), where the frame shows them.
    pub lights: crate::lights::PointLights,
}

impl Optics {
    /// The blades' polygon's share of the aperture's shape (0 round, 1 the polygon): the iris is open (round) at the
    /// widest aperture, f/1.4, and has closed to its polygon two stops down, by f/2.8.
    pub fn polygon(&self) -> f32 {
        if self.blades < 3 {
            return 0.0;
        }
        let t = ((self.aperture / WIDEST).log2() / 2f32.sqrt().log2() / 2.0).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// The radial distortion coefficient ([`distortion_of`]), 0 with the distortion off.
    pub fn distortion_k(&self) -> f32 {
        if self.distortion {
            distortion_of(self.focal_mm)
        } else {
            0.0
        }
    }
}

/// The blades' angle in the frame drawn: their polygon stands upright in the photo (a vertex at the top), and the frame
/// drawn is turned by the roll against it.
pub fn blade_angle(roll: f32) -> f32 {
    -std::f32::consts::FRAC_PI_2 + roll
}

/// The aperture's shape for the depth of field's bokeh: blades (0 round), their angle in the frame drawn, the
/// polygon's share; round without a lens.
pub fn aperture_shape(optics: Option<&Optics>, roll: f32) -> [f32; 4] {
    match optics {
        Some(optics) if optics.blades >= 3 => [
            optics.blades as f32,
            blade_angle(roll),
            optics.polygon(),
            0.0,
        ],
        _ => [0.0; 4],
    }
}

/// The display's lens terms ([`crate::display`]): the distortion's `k` and the chromatic aberration's shift at the
/// corners; none without a lens.
pub fn display_lens(optics: Option<&Optics>) -> [f32; 2] {
    match optics {
        Some(optics) => [
            optics.distortion_k(),
            chromatic_shift(optics.chromatic, optics.focal_mm),
        ],
        None => [0.0; 2],
    }
}

/// The radial distortion of a lens of `focal_mm`: `k` in `r_d = r (1 + k r²)`, `r` over the frame's half-diagonal, so
/// `k` is the shift at the corners. A little barrel (negative) on the wide lenses (−2.5 % at 14 mm, −0.8 % at 24),
/// none from 35 to 200 mm, a slight pincushion on the long ones (+0.6 % at 300, +1 % at 400).
pub fn distortion_of(focal_mm: f32) -> f32 {
    let f = focal_mm.max(1.0);
    if f < 35.0 {
        // log-linear from −2.5 % at 14 mm to 0 at 35.
        let t = ((35.0 / f).ln() / (35.0f32 / 14.0).ln()).clamp(0.0, 1.5);
        -0.025 * t
    } else if f > 200.0 {
        let t = ((f / 200.0).ln() / 2f32.ln()).clamp(0.0, 1.5);
        0.01 * t
    } else {
        0.0
    }
}

/// How much wider than the photo the frame drawn must be for a distortion `k` to find the scene at the photo's
/// corners: a barrel's corners reach out to `r` with `r (1 + k r²) = 1`; a pincushion's stay inside.
pub fn distortion_cover(k: f32) -> f32 {
    if k >= 0.0 {
        return 1.0;
    }
    undistort(1.0, k)
}

/// The undistorted radius `r` with `r (1 + k r²) = r_d` (Newton from `r_d`).
pub fn undistort(r_d: f32, k: f32) -> f32 {
    let mut r = r_d;
    for _ in 0..4 {
        let f = r * (1.0 + k * r * r) - r_d;
        let d = 1.0 + 3.0 * k * r * r;
        r -= f / d;
    }
    r
}

/// Lateral chromatic aberration's shift at the corners, over the frame's half-diagonal, for `amount` 0–1 on a lens of
/// `focal_mm`: red magnified by it, blue shrunk by it, green as drawn. Strongest on the shortest and longest lenses
/// (0.4 % at 14 and 400 mm), least about 50–85 mm (0.12 %).
pub fn chromatic_shift(amount: f32, focal_mm: f32) -> f32 {
    let f = focal_mm.max(1.0);
    let wide = ((65.0 / f).ln() / (65.0f32 / 14.0).ln()).clamp(0.0, 1.0);
    let long = ((f / 65.0).ln() / (400.0f32 / 65.0).ln()).clamp(0.0, 1.0);
    let extreme = wide.max(long);
    amount.clamp(0.0, 1.0) * (0.0012 + 0.0028 * extreme * extreme)
}

/// The share of the sun's light a lens takes in from `rho` (its image's distance from the centre over the frame's
/// half-diagonal): whole within the image circle, gone by 1.6 times it (the barrel and the hood stop it).
pub fn acceptance(rho: f32) -> f32 {
    let t = ((rho - 1.0) / 0.6).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// One ghost: an image of the aperture made by light reflected off two of the lens's surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ghost {
    /// Its centre over the source's offset from the frame's centre (negative: across the centre).
    pub along: f32,
    /// Its radius in frame heights at f/4.
    pub size: f32,
    /// The two surfaces (0–5) that reflect it.
    pub surfaces: (usize, usize),
}

/// A four-element lens's six air-glass surfaces' ghosts that come out bright enough to see (of the fifteen pairs).
pub const GHOSTS: [Ghost; 8] = [
    Ghost {
        along: -0.35,
        size: 0.10,
        surfaces: (0, 1),
    },
    Ghost {
        along: -0.62,
        size: 0.045,
        surfaces: (1, 2),
    },
    Ghost {
        along: -1.05,
        size: 0.16,
        surfaces: (0, 3),
    },
    Ghost {
        along: 0.42,
        size: 0.035,
        surfaces: (2, 3),
    },
    Ghost {
        along: -1.45,
        size: 0.26,
        surfaces: (1, 4),
    },
    Ghost {
        along: 0.72,
        size: 0.085,
        surfaces: (3, 4),
    },
    Ghost {
        along: -0.18,
        size: 0.022,
        surfaces: (4, 5),
    },
    Ghost {
        along: 1.25,
        size: 0.2,
        surfaces: (2, 5),
    },
];

/// The six surfaces' glass: crown (1.517) and flint (1.620) alternating, each with Cauchy's dispersion.
const GLASS: [(f32, f32); 6] = [
    // (A, B) of n(λ) = A + B / λ² (λ in µm).
    (1.5045, 0.00420),
    (1.6000, 0.00905),
    (1.5045, 0.00420),
    (1.6000, 0.00905),
    (1.5045, 0.00420),
    (1.6000, 0.00905),
];
/// Each surface's coating: the wavelength (nm) its layers are tuned for, which sets the colour it reflects.
const COATING_NM: [f32; 6] = [520.0, 590.0, 545.0, 610.0, 500.0, 565.0];

fn index_of(surface: usize, wavelength_mm: f32) -> f32 {
    let (a, b) = GLASS[surface];
    let um = wavelength_mm * 1e3;
    a + b / (um * um)
}

/// An uncoated surface's reflectance at normal incidence (Fresnel): `((n − 1) / (n + 1))²`, about 4.2 % on crown and
/// 5.6 % on flint.
pub fn uncoated_reflectance(surface: usize, wavelength_mm: f32) -> f32 {
    let n = index_of(surface, wavelength_mm);
    ((n - 1.0) / (n + 1.0)).powi(2)
}

/// A coated surface's reflectance: a quarter-wave layer of magnesium fluoride (n 1.38) tuned to the surface's
/// wavelength, its residue a quarter of a single layer's (the extra layers of a multi-coating), so 0.16 % (flint) to
/// 0.3 % (crown) at the tuned wavelength, rising toward the spectrum's ends: the colour of its ghosts.
pub fn coated_reflectance(surface: usize, wavelength_mm: f32) -> f32 {
    let n0 = 1.0f32;
    let n1 = 1.38f32;
    let n2 = index_of(surface, wavelength_mm);
    let r01 = (n0 - n1) / (n0 + n1);
    let r12 = (n1 - n2) / (n1 + n2);
    // The layer's phase: a quarter wave at the tuned wavelength.
    let tuned_mm = COATING_NM[surface] * 1e-6;
    let two_beta = std::f32::consts::PI * tuned_mm / wavelength_mm;
    let c = two_beta.cos();
    let single = (r01 * r01 + r12 * r12 + 2.0 * r01 * r12 * c)
        / (1.0 + r01 * r01 * r12 * r12 + 2.0 * r01 * r12 * c);
    0.25 * single
}

/// A ghost's share of its source's flux per channel: its two surfaces' reflectances multiplied.
pub fn ghost_tint(coating: Coating, ghost: &Ghost) -> Vec3 {
    let reflect = |surface, channel: usize| {
        let wavelength = WAVELENGTHS_MM[channel];
        match coating {
            Coating::Coated => coated_reflectance(surface, wavelength),
            Coating::Uncoated => uncoated_reflectance(surface, wavelength),
        }
    };
    let (a, b) = ghost.surfaces;
    Vec3::from_array(std::array::from_fn(|c| reflect(a, c) * reflect(b, c)))
}

/// The starburst's strength for a source of unit flux: the flux per pixel of length along one spike at `r` pixels
/// from the source is this over `r²`, per channel. A regular `blades`-gon with the area of a circle of the aperture's
/// diameter `D = f / N`: each edge of length `L` sends `Φ λ L / (4π² A θ²)` per radian along its normal, both ways
/// (Fraunhofer, the edge's transform falling as `1 / k`); on the sensor that is `Φ λ N (L D / A) / (4π² r²)` per
/// millimetre, `pixel_mm` millimetres a pixel. An even count's opposite edges share a spike (twice the light each).
pub fn spike_coefficient(blades: u32, aperture: f32, pixel_mm: f32) -> Vec3 {
    if blades < 3 {
        return Vec3::ZERO;
    }
    let n = blades as f32;
    let pi = std::f32::consts::PI;
    // The circumradius, in aperture diameters, of the n-gon with the circle's area.
    let a = (pi / (2.0 * n * (2.0 * pi / n).sin())).sqrt();
    let edge_over_area = 2.0 / (n * a * (pi / n).cos());
    let share = if blades.is_multiple_of(2) { 2.0 } else { 1.0 };
    Vec3::from_array(WAVELENGTHS_MM.map(|wavelength| {
        share * wavelength * aperture * edge_over_area / (4.0 * pi * pi) / pixel_mm.max(1e-6)
    }))
}

/// The number of spikes `blades` straight blades draw: as many for an even count, twice as many for an odd one.
pub fn spike_count(blades: u32) -> u32 {
    match blades {
        0..=2 => 0,
        n if n.is_multiple_of(2) => n,
        n => 2 * n,
    }
}

/// The veiling glare's kernel: level `i`'s weight (before normalising by [`glare_weight_sum`]).
pub fn glare_weight(level: usize) -> f32 {
    GLARE_FALLOFF.powi(level as i32)
}

pub fn glare_weight_sum(levels: usize) -> f32 {
    (0..levels).map(glare_weight).sum()
}

/// The glare pyramid's levels for a frame of `viewport`: its quarter halved until two pixels or so across.
pub fn glare_levels(viewport: (u32, u32)) -> usize {
    let mut size = viewport.0.max(viewport.1).div_ceil(4).max(1);
    let mut levels = 1;
    // Two levels at least: the up chain has one fewer.
    while (size > 2 || levels < 2) && levels < MAX_GLARE_LEVELS {
        size = size.div_ceil(2).max(1);
        levels += 1;
    }
    levels
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OpticsUniforms {
    /// Width, height, 1 / width, 1 / height of the frame drawn.
    frame: [f32; 4],
    /// Tiles across and down, a tile's pixels, sources (tiles + the sun's).
    tiles: [f32; 4],
    /// The display's exposure with photo mode's EV (the eye's is read on the GPU), the sources' threshold, the
    /// sprites' cutoff, the frame drawn over the photo's (the roll's widening).
    exposure: [f32; 4],
    /// Blades (0 round), the blades' angle in the frame drawn (radians), the polygon's share, the f-number.
    lens: [f32; 4],
    /// The photo's height in drawn pixels, the ghosts' size factor (`4 / N`), the share of the frame's light the lens
    /// scatters (η, 0 for a coated lens), 0.
    lens2: [f32; 4],
    /// The glare's weights: the frame's halo over its levels' weights' sum and its plateau, as shares of its η; the
    /// sun's halo η over the weights' sum and its plateau η over the image circle's area (pixels).
    veil: [f32; 4],
    /// The spikes' coefficient per channel ([`spike_coefficient`]), the spike count.
    spike: [f32; 4],
    /// The sun's image (drawn pixels), 1 in front of the camera, the lens's acceptance of it.
    sun: [f32; 4],
    /// The sun's flux per unit of what reaches the lens (irradiance over a pixel's solid angle at its place), 1 with a
    /// sun.
    sun_flux: [f32; 4],
    /// The sun's own radius (pixels), its drawn disc's (pixels), the drawn disc's unobstructed radiance (luminance),
    /// what reaches the lens of a sun out of the frame.
    sun_disc: [f32; 4],
    /// The glare pyramid's base size, its levels, its weights' sum.
    glare: [f32; 4],
    /// The camera's near and far planes (to read reversed depth as metres), the lights given, 0.
    planes: [f32; 4],
    /// The depth of field: its circle of confusion's diameter at infinity (pixels), the focus (m), 1 when on.
    focus: [f32; 4],
    /// Per point light: its image (drawn pixels), its distance along the view axis (m), its radius (pixels); the same
    /// of the top of its column ([`LIGHT_COLUMN_M`]) and 0; its flux (the frame's units, before what of it the frame
    /// hides) and 0.
    lights: [[f32; 4]; 3 * MAX_LIGHTS],
    /// Per ghost: along, size (frame heights at f/4), 0, 0.
    ghosts: [[f32; 4]; 8],
    /// Per ghost: its share of the flux per channel, 0.
    tints: [[f32; 4]; 8],
}

/// One source (`Source` in the shader): flux (RGB, the frame's radiance times pixels) and the area it covers, then
/// its centroid (drawn pixels), its radius and 1 for the sun.
const SOURCE_BYTES: u64 = 32;

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct Targets {
    viewport: (u32, u32),
    tiles: (u32, u32),
    sources: wgpu::Buffer,
    out: Target,
    version: u64,
    /// The glare's pyramid: down levels, then the up chain (one fewer).
    down: Vec<Target>,
    up: Vec<Target>,
}

/// The flare's passes and targets.
pub(crate) struct OpticsPass {
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    compute_layout: wgpu::BindGroupLayout,
    fragment_layout: wgpu::BindGroupLayout,
    sprite_layout: wgpu::BindGroupLayout,
    sources: PipelineId,
    sun: PipelineId,
    lights: PipelineId,
    glare_first: PipelineId,
    glare_down: PipelineId,
    glare_up: PipelineId,
    composite: PipelineId,
    sprites: PipelineId,
    targets: Option<Targets>,
    next_version: u64,
}

/// What the flare reads this frame.
pub(crate) struct OpticsInputs<'a> {
    /// `scene.hdr`: the sources.
    pub scene: &'a wgpu::TextureView,
    /// `scene.depth`.
    pub depth: &'a wgpu::TextureView,
    /// The frame the display would show (the depth of field's, or `scene.hdr`): the glare and the composite's base.
    pub source: &'a wgpu::TextureView,
    /// The eye adaptation's state.
    pub eye: &'a wgpu::Buffer,
    pub viewport: (u32, u32),
    /// The display's exposure with photo mode's EV.
    pub exposure: f32,
    /// The frame drawn over the photo's (the roll's widening, [`crate::photo::roll_scale`]).
    pub frame_scale: f32,
    /// The camera's roll (radians, clockwise).
    pub roll: f32,
    /// The camera the frame is drawn with, to place the sun.
    pub camera: &'a crate::Camera,
    /// The depth of field, when on: a source out of focus draws no starburst.
    pub dof: Option<crate::photo::DepthOfField>,
}

fn blend_add() -> wgpu::BlendState {
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: add,
        alpha: add,
    }
}

impl OpticsPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<Self, ShaderError> {
        library.add(PHOTO_OPTICS_SHADER)?;
        let texture = |binding, visibility, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let depth = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding, visibility, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo optics sources"),
            entries: &[
                texture(0, compute, false),
                depth(1, compute),
                uniform(2, compute),
                storage(5, compute, false),
                storage(6, compute, true),
            ],
        });
        let fragment_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo optics glare"),
            entries: &[
                texture(0, fragment, true),
                uniform(2, fragment),
                texture(3, fragment, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                storage(7, fragment, true),
            ],
        });
        let sprite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo optics sprites"),
            entries: &[
                uniform(2, both),
                storage(7, both, true),
                storage(6, both, true),
            ],
        });
        let sources = pipelines.add(
            device,
            library,
            "photo optics sources",
            PHOTO_OPTICS_SHADER.path,
            Vec::new(),
            compute_recipe(vec![compute_layout.clone()], "sources"),
        )?;
        let sun = pipelines.add(
            device,
            library,
            "photo optics sun",
            PHOTO_OPTICS_SHADER.path,
            Vec::new(),
            compute_recipe(vec![compute_layout.clone()], "sun"),
        )?;
        let lights = pipelines.add(
            device,
            library,
            "photo optics lights",
            PHOTO_OPTICS_SHADER.path,
            Vec::new(),
            compute_recipe(vec![compute_layout.clone()], "lights"),
        )?;
        let mut fullscreen = |label: &str, entry: &'static str, blend| {
            pipelines.add(
                device,
                library,
                label,
                PHOTO_OPTICS_SHADER.path,
                Vec::new(),
                fullscreen_recipe(
                    vec![fragment_layout.clone()],
                    "vertex",
                    entry,
                    FORMAT,
                    blend,
                ),
            )
        };
        let glare_first = fullscreen("photo optics glare first", "glare_first", None)?;
        let glare_down = fullscreen("photo optics glare down", "glare_down", None)?;
        let glare_up = fullscreen("photo optics glare up", "glare_up", None)?;
        let composite = fullscreen("photo optics composite", "composite", None)?;
        let sprites = pipelines.add(
            device,
            library,
            "photo optics sprites",
            PHOTO_OPTICS_SHADER.path,
            Vec::new(),
            fullscreen_recipe(
                vec![sprite_layout.clone()],
                "sprite_vertex",
                "sprite_fragment",
                FORMAT,
                Some(blend_add()),
            ),
        )?;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo optics uniforms"),
            size: std::mem::size_of::<OpticsUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("photo optics"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            uniforms,
            sampler,
            compute_layout,
            fragment_layout,
            sprite_layout,
            sources,
            sun,
            lights,
            glare_first,
            glare_down,
            glare_up,
            composite,
            sprites,
            targets: None,
            next_version: FIRST_VERSION,
        })
    }

    /// Lets the targets go (photo mode left, or its optics off).
    pub(crate) fn release(&mut self) {
        self.targets = None;
    }

    fn target(device: &wgpu::Device, label: &str, size: (u32, u32)) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Target { texture, view }
    }

    fn ensure(&mut self, device: &wgpu::Device, viewport: (u32, u32), glare: bool) {
        let fits = self.targets.as_ref().is_some_and(|targets| {
            targets.viewport == viewport && targets.down.is_empty() != glare
        });
        if fits {
            return;
        }
        let tiles = (viewport.0.div_ceil(TILE), viewport.1.div_ceil(TILE));
        let count = u64::from(tiles.0 * tiles.1) + 1 + MAX_LIGHTS as u64;
        let sources = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo optics sources"),
            size: count * SOURCE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let (mut down, mut up) = (Vec::new(), Vec::new());
        if glare {
            let mut size = (viewport.0.div_ceil(4), viewport.1.div_ceil(4));
            let levels = glare_levels(viewport);
            for level in 0..levels {
                down.push(Self::target(device, "photo optics glare down", size));
                if level + 1 < levels {
                    up.push(Self::target(device, "photo optics glare up", size));
                }
                size = (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1));
            }
        }
        self.next_version += 1;
        self.targets = Some(Targets {
            viewport,
            tiles,
            sources,
            out: Self::target(device, "photo optics out", viewport),
            version: self.next_version,
            down,
            up,
        });
    }

    /// Draws the flare of `optics` over `inputs.source` (with its ghosts and spikes when `sprites`, else the veiling
    /// glare alone: the film camera's veil in play, `crate::play_camera`); returns the result's view and version for
    /// the display.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        timer: &GpuTimer,
        inputs: &OpticsInputs<'_>,
        optics: &Optics,
        sprites: bool,
    ) -> (&wgpu::TextureView, u64) {
        let glare = optics.coating.glares_everything();
        self.ensure(device, inputs.viewport, glare);
        let uniforms = self.uniforms_for(inputs, optics);
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let targets = self.targets.as_ref().expect("just made");
        let source_count = targets.tiles.0 * targets.tiles.1 + 1 + MAX_LIGHTS as u32;

        // The sources: the frame's tiles, then the sun.
        let compute_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo optics sources"),
            layout: &self.compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(inputs.scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(inputs.depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: targets.sources.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: inputs.eye.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("photo optics sources"),
                timestamp_writes: timer.compute_pass("photo optics sources"),
            });
            pass.set_bind_group(0, &compute_group, &[]);
            pass.set_pipeline(pipelines.compute(self.sources));
            pass.dispatch_workgroups(targets.tiles.0, targets.tiles.1, 1);
            pass.set_pipeline(pipelines.compute(self.sun));
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(pipelines.compute(self.lights));
            pass.dispatch_workgroups(1, 1, 1);
        }

        let fragment_group = |base: &wgpu::TextureView, aux: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("photo optics"),
                layout: &self.fragment_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(base),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(aux),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: targets.sources.as_entire_binding(),
                    },
                ],
            })
        };
        let run = |encoder: &mut wgpu::CommandEncoder,
                   pipeline: PipelineId,
                   target: &wgpu::TextureView,
                   group: &wgpu::BindGroup,
                   label: &str,
                   timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timestamps,
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(pipeline));
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };

        // The veiling glare of the whole frame (the uncoated lens): a pyramid down from the frame at a quarter of its
        // size, then up again, each level weighted against the next finer.
        let glare_view = if glare {
            let levels = targets.down.len();
            let first = fragment_group(inputs.source, inputs.source);
            run(
                encoder,
                self.glare_first,
                &targets.down[0].view,
                &first,
                "photo optics glare",
                timer.render_pass("photo optics glare"),
            );
            for level in 1..levels {
                let group =
                    fragment_group(&targets.down[level - 1].view, &targets.down[level - 1].view);
                run(
                    encoder,
                    self.glare_down,
                    &targets.down[level].view,
                    &group,
                    "photo optics glare down",
                    None,
                );
            }
            for level in (0..levels - 1).rev() {
                let coarser = if level + 1 == levels - 1 {
                    &targets.down[level + 1].view
                } else {
                    &targets.up[level + 1].view
                };
                let group = fragment_group(&targets.down[level].view, coarser);
                run(
                    encoder,
                    self.glare_up,
                    &targets.up[level].view,
                    &group,
                    "photo optics glare up",
                    None,
                );
            }
            Some(&targets.up[0].view)
        } else {
            None
        };

        // The composite: the frame less what the lens scatters, the glare it scatters back, the sun's glare.
        let composite = fragment_group(inputs.source, glare_view.unwrap_or(inputs.source));
        let mut chain = timer.render_pass("photo optics flare");
        let (begin, end) = match chain.take() {
            Some(writes) => (
                Some(wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
                    end_of_pass_write_index: None,
                }),
                Some(wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: writes.end_of_pass_write_index,
                }),
            ),
            None => (None, None),
        };
        run(
            encoder,
            self.composite,
            &targets.out.view,
            &composite,
            "photo optics composite",
            begin,
        );

        // The ghosts and the spikes, added onto it.
        let sprite_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo optics sprites"),
            layout: &self.sprite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: targets.sources.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: inputs.eye.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("photo optics sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.out.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: end,
                ..Default::default()
            });
            if sprites {
                pass.set_pipeline(pipelines.render(self.sprites));
                pass.set_bind_group(0, &sprite_group, &[]);
                pass.draw(0..SPRITES * 6, 0..source_count);
            }
        }
        let _ = &targets.out.texture;
        (&targets.out.view, targets.version)
    }

    fn uniforms_for(&self, inputs: &OpticsInputs<'_>, optics: &Optics) -> OpticsUniforms {
        let (width, height) = (
            inputs.viewport.0.max(1) as f32,
            inputs.viewport.1.max(1) as f32,
        );
        let tiles = (
            inputs.viewport.0.div_ceil(TILE),
            inputs.viewport.1.div_ceil(TILE),
        );
        let frame_scale = inputs.frame_scale.max(1.0);
        let photo_height = height / frame_scale;
        // Millimetres of the sensor a drawn pixel spans (24 mm over the photo's height).
        let pixel_mm = 24.0 / photo_height;
        let aperture = optics.aperture.max(0.5);
        let polygon = optics.polygon();
        let spike = spike_coefficient(optics.blades, aperture, pixel_mm) * polygon;
        let blade_angle = blade_angle(inputs.roll);
        let mut ghosts = [[0.0; 4]; 8];
        let mut tints = [[0.0; 4]; 8];
        for (index, ghost) in GHOSTS.iter().enumerate() {
            ghosts[index] = [ghost.along, ghost.size, 0.0, 0.0];
            tints[index] = ghost_tint(optics.coating, ghost).extend(0.0).to_array();
        }
        let levels = glare_levels(inputs.viewport);
        let weights = glare_weight_sum(levels);
        let (frame_halo, frame_plateau) = optics.coating.frame_glare();
        let frame_eta = frame_halo + frame_plateau;
        let (sun_halo, sun_plateau) = optics.coating.sun_glare();
        // The image circle: the photo's half-diagonal.
        let circle = 0.5 * (width * width + height * height).sqrt() / frame_scale;
        let circle_area = std::f32::consts::PI * circle * circle;
        let (sun, sun_flux, sun_disc) = self.sun_terms(inputs, optics, (width, height));
        let glare_base = (
            inputs.viewport.0.div_ceil(4) as f32,
            inputs.viewport.1.div_ceil(4) as f32,
        );
        OpticsUniforms {
            frame: [width, height, 1.0 / width, 1.0 / height],
            tiles: [
                tiles.0 as f32,
                tiles.1 as f32,
                TILE as f32,
                (tiles.0 * tiles.1 + 1) as f32 + MAX_LIGHTS as f32,
            ],
            exposure: [inputs.exposure, SOURCE_THRESHOLD, CUTOFF, frame_scale],
            lens: [optics.blades as f32, blade_angle, polygon, aperture],
            lens2: [photo_height, GHOST_APERTURE / aperture, frame_eta, 0.0],
            veil: [
                if frame_eta > 0.0 {
                    frame_halo / frame_eta / weights
                } else {
                    0.0
                },
                if frame_eta > 0.0 {
                    frame_plateau / frame_eta
                } else {
                    0.0
                },
                sun_halo / weights,
                sun_plateau / circle_area,
            ],
            spike: [spike.x, spike.y, spike.z, spike_count(optics.blades) as f32],
            sun,
            sun_flux,
            sun_disc,
            glare: [glare_base.0, glare_base.1, levels as f32, 0.0],
            planes: [
                inputs.camera.near,
                inputs.camera.far,
                optics.lights.len() as f32,
                0.0,
            ],
            lights: Self::light_terms(inputs, optics, (width, height)),
            focus: match inputs.dof {
                Some(dof) => [dof.coc_scale(height), dof.focus_m.max(1e-3), 1.0, 0.0],
                None => [0.0; 4],
            },
            ghosts,
            tints,
        }
    }

    /// The point lights' terms: where each one's image falls in the frame drawn, how far along the view axis it is,
    /// its radius, and its flux (its candela over the distance squared, over a pixel's solid angle there), none
    /// behind the camera or beyond the lens's field.
    fn light_terms(
        inputs: &OpticsInputs<'_>,
        optics: &Optics,
        (width, height): (f32, f32),
    ) -> [[f32; 4]; 3 * MAX_LIGHTS] {
        let mut terms = [[0.0; 4]; 3 * MAX_LIGHTS];
        let camera = inputs.camera;
        let view = camera.view();
        let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-6);
        let aspect = width / height;
        let half_diagonal = (aspect * aspect + 1.0).sqrt() / inputs.frame_scale.max(1.0);
        let pixel = 2.0 * tan / height;
        for (index, light) in optics.lights.as_slice().iter().enumerate().take(MAX_LIGHTS) {
            // Where the curved world draws it (`crate::camera::curved`), with its flames and flash.
            let p = view.transform_point3(camera.curved(light.position));
            let depth = -p.z;
            if depth <= 1.0 {
                continue;
            }
            let (x, y) = (p.x / depth / tan, p.y / depth / tan);
            let rho = (x * x + y * y).sqrt() / half_diagonal;
            let accept = acceptance(rho);
            if accept <= 0.0 {
                continue;
            }
            let distance = p.length();
            let irradiance =
                light.color * light.intensity / distance.powf(light.decay.max(0.0)).max(1.0);
            let cos = depth / distance;
            let solid = pixel * pixel * cos * cos * cos;
            let flux = irradiance * (accept / solid.max(1e-12));
            let image = |p: Vec3| {
                let depth = -p.z.min(-1e-3);
                let (x, y) = (p.x / depth / tan, p.y / depth / tan);
                (
                    width * 0.5 * (1.0 + x / aspect),
                    height * 0.5 * (1.0 - y),
                    depth,
                )
            };
            let base = image(p);
            let top = image(
                view.transform_point3(camera.curved(light.position + Vec3::Y * LIGHT_COLUMN_M)),
            );
            let radius = (0.5 * LIGHT_SIZE_M / depth / pixel).max(1.0);
            terms[3 * index] = [base.0, base.1, base.2, radius];
            terms[3 * index + 1] = [top.0, top.1, top.2, 0.0];
            terms[3 * index + 2] = [flux.x, flux.y, flux.z, 0.0];
        }
        terms
    }

    /// The sun's terms: where its image falls in the frame drawn, its flux per unit of what reaches the lens, its
    /// discs' radii.
    fn sun_terms(
        &self,
        inputs: &OpticsInputs<'_>,
        optics: &Optics,
        (width, height): (f32, f32),
    ) -> ([f32; 4], [f32; 4], [f32; 4]) {
        let none = ([0.0; 4], [0.0; 4], [0.0; 4]);
        let Some(sun) = optics.sun else {
            return none;
        };
        let camera = inputs.camera;
        let view = camera.view();
        // The direction in the camera's frame (looking down −Z).
        let d = view.transform_vector3(sun.direction.normalize_or_zero());
        if d.z >= -1e-4 || sun.irradiance.max_element() <= 0.0 {
            return none;
        }
        let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-6);
        let aspect = width / height;
        // On the image plane, in units of its half-height, then in pixels.
        let (x, y) = (d.x / -d.z / tan, d.y / -d.z / tan);
        let px = (width * 0.5 * (1.0 + x / aspect), height * 0.5 * (1.0 - y));
        // Its distance from the centre over the photo's half-diagonal.
        let frame_scale = inputs.frame_scale.max(1.0);
        let half_diagonal = (aspect * aspect + 1.0).sqrt() / frame_scale;
        let rho = (x * x + y * y).sqrt() / half_diagonal;
        let accept = acceptance(rho);
        if accept <= 0.0 {
            return none;
        }
        // A pixel's solid angle at the centre and at the sun's place (cos³ off the axis).
        let pixel = 2.0 * tan / height;
        let cos = -d.z / d.length().max(1e-6);
        let solid = pixel * pixel * cos * cos * cos;
        let flux = sun.irradiance / solid.max(1e-12);
        let radius_px = sun.angular_radius / pixel;
        let drawn_px = sun.drawn_radius / pixel;
        let disc = sun.disc_radiance.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        (
            [px.0, px.1, 1.0, accept],
            [flux.x, flux.y, flux.z, 1.0],
            [radius_px, drawn_px, disc, 1.0],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distortion_follows_the_focal_length() {
        assert!(
            (distortion_of(14.0) + 0.025).abs() < 1e-4,
            "barrel at 14 mm"
        );
        let at24 = distortion_of(24.0);
        assert!(
            at24 < -0.005 && at24 > -0.012,
            "a little barrel at 24 mm: {at24}"
        );
        for f in [35.0, 50.0, 85.0, 135.0, 200.0] {
            assert_eq!(distortion_of(f), 0.0, "{f} mm is straight");
        }
        assert!(
            (distortion_of(400.0) - 0.01).abs() < 1e-4,
            "pincushion at 400 mm"
        );
        let at300 = distortion_of(300.0);
        assert!(at300 > 0.004 && at300 < 0.007, "{at300}");
        // A barrel's corners reach past the photo's, a pincushion's do not.
        let k = distortion_of(14.0);
        let cover = distortion_cover(k);
        assert!(cover > 1.02 && cover < 1.03, "{cover}");
        assert!((cover * (1.0 + k * cover * cover) - 1.0).abs() < 1e-5);
        assert_eq!(distortion_cover(0.01), 1.0);
        assert!((undistort(0.5, 0.01) * (1.0 + 0.01 * 0.25f32.powi(1)) - 0.5).abs() < 2e-3);
    }

    #[test]
    fn chromatic_aberration_is_strongest_on_the_extremes() {
        let wide = chromatic_shift(1.0, 14.0);
        let normal = chromatic_shift(1.0, 65.0);
        let long = chromatic_shift(1.0, 400.0);
        assert!((wide - 0.004).abs() < 1e-5 && (long - 0.004).abs() < 1e-5);
        assert!((normal - 0.0012).abs() < 1e-6);
        assert_eq!(chromatic_shift(0.0, 14.0), 0.0);
        assert!((chromatic_shift(0.5, 14.0) - wide * 0.5).abs() < 1e-7);
    }

    #[test]
    fn coatings_reflect_a_tenth_of_bare_glass() {
        for surface in 0..6 {
            for wavelength in WAVELENGTHS_MM {
                let bare = uncoated_reflectance(surface, wavelength);
                let coated = coated_reflectance(surface, wavelength);
                assert!(
                    (0.04..0.06).contains(&bare),
                    "bare glass reflects 4-6 %: {bare}"
                );
                assert!(
                    (0.001..0.006).contains(&coated),
                    "a coated surface a few tenths: {coated}"
                );
            }
        }
        // A ghost through an uncoated lens is a hundred times one through a coated one or more.
        for ghost in &GHOSTS {
            let bare = ghost_tint(Coating::Uncoated, ghost);
            let coated = ghost_tint(Coating::Coated, ghost);
            assert!(
                bare.min_element() / coated.max_element() > 60.0,
                "{bare} {coated}"
            );
            // The coating colours its ghosts; bare glass barely.
            let spread = |v: Vec3| v.max_element() / v.min_element();
            assert!(spread(coated) > 1.1, "{coated}");
            assert!(spread(bare) < 1.1, "{bare}");
        }
    }

    #[test]
    fn the_starburst_has_a_spike_per_edge_normal() {
        assert_eq!(spike_count(0), 0);
        assert_eq!(spike_count(5), 10);
        assert_eq!(spike_count(6), 6);
        assert_eq!(spike_count(8), 8);
        assert_eq!(spike_coefficient(0, 8.0, 0.02), Vec3::ZERO);
        // Stopping down lengthens the spikes in proportion (their flux at a distance), red further than blue.
        let open = spike_coefficient(6, 4.0, 0.02);
        let shut = spike_coefficient(6, 16.0, 0.02);
        assert!((shut.y / open.y - 4.0).abs() < 1e-4);
        assert!(open.x > open.y && open.y > open.z);
        // A hexagon's: 2 λ N (L D / A) / (4π²) per millimetre at a millimetre, L D / A = 2 / (n a cos(π/n)).
        let n = 6.0f32;
        let pi = std::f32::consts::PI;
        let a = (pi / (2.0 * n * (2.0 * pi / n).sin())).sqrt();
        let expect = 2.0 * 550e-6 * 4.0 * (2.0 / (n * a * (pi / n).cos())) / (4.0 * pi * pi) / 0.02;
        assert!((open.y - expect).abs() < expect * 1e-4);
    }

    #[test]
    fn the_iris_closes_from_round() {
        let lens = |blades, aperture| Optics {
            coating: Coating::Coated,
            blades,
            aperture,
            focal_mm: 50.0,
            chromatic: 0.0,
            distortion: false,
            sun: None,
            lights: crate::lights::PointLights::NONE,
        };
        assert_eq!(lens(6, 1.4).polygon(), 0.0, "wide open it is round");
        assert_eq!(
            lens(6, 2.8).polygon(),
            1.0,
            "a stop and more down it is the polygon"
        );
        assert_eq!(lens(0, 8.0).polygon(), 0.0, "a round aperture stays round");
        let between = lens(5, 2.0).polygon();
        assert!(between > 0.2 && between < 0.8, "{between}");
    }

    #[test]
    fn the_sun_fades_beyond_the_image_circle() {
        assert_eq!(acceptance(0.0), 1.0);
        assert_eq!(acceptance(1.0), 1.0);
        assert_eq!(acceptance(1.6), 0.0);
        assert!(acceptance(1.3) > 0.4 && acceptance(1.3) < 0.6);
    }

    #[test]
    fn the_glare_kernel_falls_off_by_octaves() {
        let sum = glare_weight_sum(8);
        assert!((glare_weight(0) - 1.0).abs() < 1e-6);
        assert!((glare_weight(2) - 0.5).abs() < 1e-6);
        assert!(sum > 3.0 && sum < 3.3, "{sum}");
        // Down to two pixels or so: 2592 wide is 648 at its base, nine halvings on.
        assert_eq!(glare_levels((2592, 1545)), 10);
        assert_eq!(glare_levels((1600, 900)), 9);
        // A tiny frame still has an up chain.
        assert_eq!(glare_levels((8, 8)), 2);
        assert_eq!(glare_levels((1, 1)), 2);
        // The uncoated lens scatters six times the coated lens's share of the sun.
        let total = |(a, b): (f32, f32)| a + b;
        assert!(
            (total(Coating::Uncoated.sun_glare()) / total(Coating::Coated.sun_glare()) - 6.0).abs()
                < 1e-5
        );
        assert!(!Coating::Coated.glares_everything() && Coating::Uncoated.glares_everything());
    }
}
