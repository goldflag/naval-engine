//! The land of a battle map as a renderer-core layer (the web game's `BattleLandscape.ts`'s `HeightfieldLandscape`): the
//! field's textures, the open sky baked once on the GPU, the level-of-detail patches chosen for the camera every frame,
//! the land (and its trees) drawn in the `Opaque` phase, the land's own shadow recomputed when the sun moves (`sunlight`,
//! on the host's worker when it hands one), and the land and its trees in the sun's shadow maps.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use naval_ocean::FogParameters;
use naval_render::{
    CasterPipeline, DrawContext, Phase, PipelineId, PrepareContext, RenderContext, SceneLayer,
    ScenePipeline, ShaderDefValue, ShaderError, ShadowMap, ShadowMapKind, VertexLayout,
    compute_recipe, shader_file,
};
use naval_sky::CelestialLight;

use crate::dfg::{DFG_LUT, DFG_SIZE};
use crate::field::{ATTRIBUTE_STRIDE, PreparedField};
use crate::heightfield::Heightfield;
use crate::land::PreparedLand;
use crate::lod::{
    LodView, PATCH_CAPACITY, PATCH_FLOATS, PATCH_QUADS, TerrainChart, TerrainLod, TerrainQuality,
};
use crate::noise::NOISE_SIZE;
use crate::sunlight::{LandWorker, Sunlight};
use crate::textures::{DETAIL_SIZE, mips_rgba8, pack_detail, texture_with_levels, write_levels};
use crate::trees::{
    TREE_CAPACITY, TREE_CARD_INDICES, TREE_FULL_M, TREE_GROWTH, TREE_INSTANCE_BYTES,
    TREE_LEAN_MOST, TREE_LENS_TAN, TREE_PALM_LEAN_MOST, TREE_RANGE_M, TREE_SLOTS, tree_card,
};
use naval_conditions::{LandStyle, MapScene, SurfaceStyle, mesh_light_shares};

/// Most patches the land draws into the sun's shadow maps together.
const CASTER_CAPACITY: usize =
    crate::land_shadows::LAND_MAP_PATCHES * naval_render::shadows::MAP_COUNT;
/// Land lower than this above the sea (m) counts as none for the sun's land maps (`land_about`): a beach's last metre.
const LAND_ABOVE_SEA_M: f32 = 1.5;
/// The pixel footprint (m) over which the ground materials fade out (`materials.wgsl`): fully drawn to 0.5 m, gone by
/// 1.6 m, about 1.5 km at the game's 52° lens on a 900-pixel-high view.
const MATERIAL_FADE_M: [f32; 2] = [0.5, 1.6];
/// How strongly the ground materials show by default (`set_ground_material_strength`).
pub const MATERIAL_STRENGTH: f32 = 1.3;

/// What the land can draw instead of the lit land, for review (`TERRAIN_DEBUG`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerrainDebug {
    #[default]
    Off,
    Albedo,
    Normal,
    Sun,
    Sky,
    Occlusion,
    Lod,
    Coast,
    Relief,
    Slope,
    /// The two ground materials read, each by its hue and weight (`materials.wgsl`).
    Material,
}

impl TerrainDebug {
    pub const ALL: [TerrainDebug; 11] = [
        TerrainDebug::Off,
        TerrainDebug::Albedo,
        TerrainDebug::Normal,
        TerrainDebug::Sun,
        TerrainDebug::Sky,
        TerrainDebug::Occlusion,
        TerrainDebug::Lod,
        TerrainDebug::Coast,
        TerrainDebug::Relief,
        TerrainDebug::Slope,
        TerrainDebug::Material,
    ];

    /// By name (`off`, `albedo`, `normal`, `sun`, `sky`, `occlusion`, `lod`, `coast`, `relief`, `slope`, `material`).
    pub fn by_name(name: &str) -> Option<Self> {
        let names = [
            "off",
            "albedo",
            "normal",
            "sun",
            "sky",
            "occlusion",
            "lod",
            "coast",
            "relief",
            "slope",
            "material",
        ];
        names.iter().position(|n| *n == name).map(|k| Self::ALL[k])
    }

    fn index(self) -> f32 {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0) as f32
    }
}

/// The window's satellite photograph as the land holds it ([`TerrainLayer::surface_photo`]).
#[derive(Clone, Debug)]
pub struct SurfacePhoto {
    pub view: wgpu::TextureView,
    pub origin: [f64; 2],
    pub extent_m: f64,
    pub texels: u32,
    pub haze: [f32; 3],
    pub version: u64,
}

/// How the land draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainSettings {
    /// The game's terrain setting (level-of-detail spacing, trees, detailed surface).
    pub quality: TerrainQuality,
    /// Draw the land into the sun's near and wide shadow maps, so it shades the ships, the sea, its trees and
    /// buildings (native/docs/terrain/shadows.md, "The land's shadow"); its own surface keeps its baked shadow. Off by
    /// default, as the web's land cast none: the client turns it on with the sun's maps.
    pub cast_shadows: bool,
    /// With `cast_shadows`, into the view maps too (squares about hulls the wide map misses, kilometres off).
    pub cast_view_maps: bool,
    pub debug: TerrainDebug,
}

impl Default for TerrainSettings {
    fn default() -> Self {
        Self {
            quality: TerrainQuality::HIGH,
            cast_shadows: false,
            cast_view_maps: true,
            debug: TerrainDebug::Off,
        }
    }
}

/// The light the land takes each frame: the sky's celestial light, the scene's authored ambient (the hemisphere
/// fill before the meshes' share), lightning's flash, and the environment's intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainLight {
    pub light: CelestialLight,
    pub ambient: f32,
    pub flash: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TerrainUniforms {
    field: [f32; 4],
    grid: [f32; 4],
    params: [f32; 4],
    offset: [f32; 4],
    sun_direction: [f32; 4],
    sun_radiance: [f32; 4],
    fill_sky: [f32; 4],
    fill_ground: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    shore: [f32; 4],
    low: [f32; 4],
    high: [f32; 4],
    equator: [f32; 4],
    clearing: [f32; 4],
    climate: [f32; 4],
    surface: [f32; 4],
    surface_haze: [f32; 4],
    cover_means: [[f32; 4]; crate::surface_maps::COVER_CLASSES],
    near: [f32; 4],
    near_slots: [[i32; 4]; 32],
    features: [f32; 4],
    feature_origin: [f32; 4],
    feature_slots: [[i32; 4]; 64],
    materials: [f32; 4],
    material_table: [[f32; 4]; crate::ground_materials::MATERIAL_SLOTS],
    /// The cliffs' rock: its grid about the eye and the rocks' rows (`ground_materials::cliff_grid`, `cliff_rocks`).
    cliff_corner: [f32; 4],
    cliff_grid: [[u32; 4]; crate::ground_materials::CLIFF_GRID_WORDS],
    cliff_rocks: [[f32; 4]; crate::ground_materials::CLIFF_ROCK_ROWS],
    /// The near trees' meshes: the eye (world) and the largest reach (m, 0 with none); their levels of detail
    /// (`bindings.wgsl`).
    tree_near: [f32; 4],
    tree_lod: [f32; 4],
    /// The wind in the trees: the way it blows (world x, z), its speed (m/s), the scene's clock (s).
    wind: [f32; 4],
    /// The lights on the land at night (`night`, `NightLights::uniforms`).
    night: [f32; 4],
    night_rule: [f32; 4],
    /// x, z: the theatre point at chart (0, 0) modulo the street lights' lattice (`night::LATTICE_M`).
    night_grid: [f32; 4],
    /// The fine relief's pages (`relief`): page (0, 0)'s chart corner, a texel (m), the slots a side (0 with none); the
    /// page each layer holds (two a vector; `relief::NO_PAGE` for none).
    relief: [f32; 4],
    relief_slots: [[i32; 4]; 32],
    /// Which layers hold a real page (`ReliefPage::real`): layer k at bit k mod 32 of entry k / 32.
    relief_real: [u32; 4],
    /// The period's country (`country::CountryLook::uniform`): its index, the months, a test's marks, 1 while drawn.
    country: [f32; 4],
    /// The bases' clearings (`TerrainLayer::set_clearings`, `bindings.wgsl`'s `cleared`): the groups held and the
    /// edge's width (m); each group's box (chart x, z min and max) and its capsules' first and count (two groups an
    /// entry); each capsule's ends (chart x, z) and its radius (four an entry).
    clearing_head: [f32; 4],
    clearing_boxes: [[f32; 4]; CLEARING_GROUPS],
    clearing_ranges: [[u32; 4]; CLEARING_GROUPS / 2],
    clearing_capsules: [[f32; 4]; CLEARING_CAPSULES],
    clearing_radii: [[f32; 4]; CLEARING_CAPSULES / 4],
    /// The forest floor's needles (`ground_materials::needle_floor`): their layer, period, normal strength, contrast.
    needles: [f32; 4],
    /// The trees' biomes about the eye for the forest floor (`surface.wgsl`'s `floor_at`): the trees' own grid
    /// (`TreeView::region_grid`, `regions`), copied as it moves.
    biome_grid: [f32; 4],
    biomes: [[u32; 4]; crate::species::REGION_CELLS * crate::species::REGION_CELLS / 16],
    /// Where the chart's origin stands on the earth (`TreeView::place`), for the forest floor's boreal woods (the
    /// spruce's northern limit and the birch belt by latitude: `surface.wgsl`'s `place_at`).
    place: [f32; 4],
}

/// The most groups of clearings the land holds (a base's each), and capsules in all (`TerrainLayer::set_clearings`).
pub const CLEARING_GROUPS: usize = 64;
pub const CLEARING_CAPSULES: usize = 640;

/// A piece of a clearing (chart metres): no tree within `radius` of the segment from `a` to `b`, the land cleared to
/// bare earth, or with `soft` its own ground left, worn in patches (what stands among the trees: a camp's huts and
/// roads, a dump, a taxiway in the jungle; bare earth wins where they overlap).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearingCapsule {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub radius: f32,
    pub soft: bool,
}

/// A group of a clearing's capsules and the rectangle of chart they are tested in (`min`, `max`; a fragment outside
/// it, by more than the edge, tests none of them): a tile of a base, holding the capsules that reach into it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearingGroup {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub capsules: Vec<ClearingCapsule>,
    /// A temperate base's (Europe's): its cleared ground mown grass, its trodden ground muddy grass, rather than the
    /// tropics' bare earth and coral dust.
    pub temperate: bool,
}

/// What the trees' placement reads of the view (`tree_place.wgsl`'s `TreeView`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TreeView {
    /// The frustum in chart metres, inside where n · p + w ≥ 0.
    planes: [[f32; 4]; 6],
    /// The eye in chart metres; w: the lens's zoom (the square root of tan(fov / 2) over the game's 52° lens's).
    eye: [f32; 4],
    /// The full-density distance (apparent m), the range's end (m), the most a thinned tree grows, level 0's range (m).
    lod: [f32; 4],
    /// The sun's near, wide and aboard maps (world to the light's clip space) the trees cast into this frame.
    maps: [[f32; 16]; 3],
    /// 1 where the trees cast into the near, the wide and the aboard map.
    casts: [f32; 4],
    /// The first shadow-only patch in the patch buffer, and the drawn patches carrying trees.
    shadow: [f32; 4],
    /// The biome (`species::Biome`), the ground cover's reach (m), its grid's first cell (x, z).
    biome: [f32; 4],
    /// The ground cover's grid: cells a side, a cell (m).
    cover: [f32; 4],
    /// Where the chart's origin stands on the earth (`Climate::place`): latitude and longitude (degrees), and 1 with a
    /// place.
    place: [f32; 4],
    /// Per species: the model's radius over its height, its height (m), its winter self (-1 for none).
    species: [[f32; 4]; SPECIES_SLOTS],
    /// The canopy height map's pages (`canopy`): page (0, 0)'s chart corner (x, z), a texel (m), the slots a side (0
    /// with none).
    canopy: [f32; 4],
    /// The page each canopy slot holds (two a vector; `canopy::NO_PAGE` for none).
    canopy_slots: [[i32; 4]; 32],
    /// The biomes about the eye (`species::region_grid`), a byte a cell.
    regions: [[u32; 4]; crate::species::REGION_CELLS * crate::species::REGION_CELLS / 16],
    /// The biomes' grid: its first cell's chart corner (x, z), a cell (m), 1 with a grid (a theatre's land).
    region_grid: [f32; 4],
}

/// Room for the species in the trees' view (`tree_place.wgsl`'s `TreeView::species`).
const SPECIES_SLOTS: usize = 32;
const _: () = assert!(crate::species::Species::ALL.len() <= SPECIES_SLOTS);
/// The ground cover's reach about the camera (m) and its cell (m): shrubs within a few hundred metres, a cell each.
const COVER_REACH_M: f32 = 360.0;
const COVER_CELL_M: f32 = 3.6;

/// Each species' model radius, height and winter self, as the placement reads them.
fn species_table() -> [[f32; 4]; SPECIES_SLOTS] {
    use crate::species::{Species, models};
    let mut table = [[0.0; 4]; SPECIES_SLOTS];
    for (k, model) in models().iter().enumerate().take(SPECIES_SLOTS) {
        let species = model.species;
        let winter = species
            .bare()
            .and_then(|w| Species::ALL.iter().position(|s| *s == w))
            .map_or(-1.0, |i| i as f32);
        table[k] = [model.radius, species.height_m(), winter, 0.0];
    }
    table
}

/// Most level-0 patches the trees' shadow-only placement adds (inside the sun's maps, out of view).
const TREE_SHADOW_PATCHES: usize = 256;
/// The open sky's texture, which carries the forest's shade in its green and blue with trees (`tree_shade.wgsl`).
const OPEN_SKY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The land of one battle map. Its terrain setting (`TerrainSettings::quality`) is fixed when it is made: the level
/// of detail, the surface's shader defs and the trees follow it, so another setting means another layer (the web
/// rebuilds its land too).
pub struct TerrainLayer {
    settings: TerrainSettings,
    /// Shared with the land's own shadow computed on the host's worker (`sunlight`).
    prepared: Arc<PreparedField>,
    style: SurfaceStyle,
    offset: [f64; 2],
    fog: FogParameters,
    lod: TerrainLod,
    patches: Vec<f32>,
    patch_count: usize,
    /// Patches with land at the front of `patches` that carry trees: level 0's, then level 1's.
    tree_patches: usize,
    tree_range: f32,
    tree_view: TreeView,
    /// The biomes' grid the trees' view holds: its first cell and the place it was worked out for.
    region_key: Option<([i32; 2], [u32; 2])>,
    /// The cliffs' rock grid the uniforms hold: its first cell and the place it was worked out for.
    cliff_key: Option<([i32; 2], [u32; 2])>,
    /// A place's rock faces are drawn (`show_rock_faces`; on unless hidden for a comparison).
    rock_faces: bool,
    /// The trees must be placed again this frame (the view, the clearing or the surface maps changed).
    trees_stale: bool,
    /// The grass's view, and whether it must be placed again (the view moved, or what it stands on changed).
    grass_view: crate::grass::GrassView,
    grass_stale: bool,
    /// The period's country drawn on marked land (`set_country`).
    country: Option<crate::CountryLook>,
    /// The trees cast into the sun's maps when the maps are on (`set_tree_shadows`).
    tree_shadows: bool,
    /// The maps the trees cast into this frame, as they were placed for them.
    tree_casts: crate::tree_shadows::TreeCasts,
    /// Whether the placed trees cast into the sun's land maps this frame (native/docs/render-core/shadows.md, "Sun
    /// shadows"): the trees in view and just beyond its sunward sides (`tree_margins`), whatever maps they were placed
    /// for.
    trees_cast_land: bool,
    /// The view's frustum in chart metres as the trees' placement takes it before `tree_margins` (`select`).
    tree_planes: [[f32; 4]; 6],
    /// How far (m) the trees' placement moves the view's four sides out while the trees cast into the land maps: past a
    /// side toward the sun, a tree's shadow reaches into the view (`tree_shadow_margins`).
    tree_margins: [f32; 4],
    /// Shadow-only level-0 patches after the drawn ones in `patches` (inside the maps, out of view).
    tree_shadow_patches: usize,
    /// The trees near the camera are drawn as meshes (`set_near_trees`), and what their binning reads this view.
    near_trees: bool,
    /// The trees' parts left out of the scene for a measurement (`show_tree_part`), one bit a [`TreePart`].
    hidden_parts: u8,
    near_view: crate::near_trees::NearView,
    /// The eye (world) the near trees were binned from.
    near_eye: [f32; 3],
    /// The wind in the trees (`set_wind`): the way it blows (world x, z) and its speed (m/s).
    wind: [f32; 3],
    /// The scene's clock at the last `prepare` (s), which the trees sway by.
    wind_clock: f32,
    last_view: Option<[f32; 36]>,
    uniforms: TerrainUniforms,
    uniform_buffer: wgpu::Buffer,
    grid_vertices: wgpu::Buffer,
    grid_indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// The level-0 patches with land drawn dense over the fine relief (`lod::DENSE_LAND`).
    dense: DensePatches,
    caster_instances: wgpu::Buffer,
    /// Each shadow map's patches in `caster_instances` this frame (first, count), by `ShadowMapKind::index`.
    caster_ranges: [(u32, u32); naval_render::shadows::MAP_COUNT],
    /// The land's own shadow and the light it was computed for (`sunlight`).
    sunlight: Sunlight,
    detail_texture: wgpu::Texture,
    terrain_group: wgpu::BindGroup,
    group_parts: GroupParts,
    /// The real-world surface's textures: the window's maps and the near tiles (`surface_textures`).
    surface: crate::surface_textures::SurfaceTextures,
    /// Changes whenever the surface's textures or the window's maps do ([`TerrainLayer::surface_photo`]).
    surface_version: u64,
    /// The host's number for the marks it laid over the window's cover (`set_cover_marks`).
    cover_marks: u64,
    /// Map features on the ground: roads, railways and waterways as distance fields about the camera (`features`).
    features: crate::features::FeatureTextures,
    /// The canopy height map's pages about the camera, which the trees' placement stands the trees on (`canopy`).
    canopy: crate::canopy::CanopyTextures,
    /// The fine relief's pages about the camera (`relief`).
    relief: crate::relief::ReliefTextures,
    /// The ground materials' array (`ground_materials`), and whether materials are handed in and shown.
    ground: GroundTextures,
    /// The host's ground materials this land was given, held (not read) while it draws, so a host that keeps only a
    /// weak handle to them hands the next land the same copy and lets it go when no land draws (`hold_ground_materials`).
    ground_held: Option<Arc<crate::GroundMaterials>>,
    /// The camera's chart point at the last `prepare` (x, z).
    camera_chart: Option<[f64; 2]>,
    bake_group: wgpu::BindGroup,
    bake_inputs: wgpu::BindGroup,
    environment: wgpu::BindGroup,
    cloud_layout: wgpu::BindGroupLayout,
    cloud_group: wgpu::BindGroup,
    cloud_placeholder: wgpu::BindGroup,
    cloud_bound: bool,
    land: PipelineId,
    caster: PipelineId,
    bake: PipelineId,
    baked: bool,
    open_sky: wgpu::Texture,
    trees: Option<TreeDraw>,
    /// The lights on the land at night (`night`): the High setting's.
    lamps: Option<crate::night::NightLamps>,
    night_lights: crate::night::NightLights,
    /// How dark it is for the lights, 0-1 (`night::night_of`).
    night: f32,
    /// The lights at night drawn (the default); hidden, the land is drawn as with none (for comparisons).
    night_shown: bool,
}

/// What the terrain bind group is made of, kept to make it again when the surface maps change.
struct GroupParts {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// Bindings 1-7, 10 and 11, in order.
    views: [wgpu::TextureView; 9],
    clamp_sampler: wgpu::Sampler,
    repeat_sampler: wgpu::Sampler,
    /// Binding 14 (map features) is in the layout.
    features: bool,
    /// Binding 15 (the ground materials) is in the layout.
    materials: bool,
    /// Bindings 16 and 17 (the fine relief's heights and slopes) are in the layout.
    relief: bool,
}

impl GroupParts {
    /// The terrain bind group with these surface maps (colour, cover) at bindings 12 and 13, map features at 14, the
    /// ground materials at 15 and the fine relief's heights and slopes at 16 and 17.
    fn bind_group(
        &self,
        device: &wgpu::Device,
        albedo: &wgpu::TextureView,
        cover: &wgpu::TextureView,
        features: &wgpu::TextureView,
        materials: &wgpu::TextureView,
        relief: (&wgpu::TextureView, &wgpu::TextureView),
    ) -> wgpu::BindGroup {
        fn texture(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            }
        }
        let v = &self.views;
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniforms.as_entire_binding(),
            },
            texture(1, &v[0]),
            texture(2, &v[1]),
            texture(3, &v[2]),
            texture(4, &v[3]),
            texture(5, &v[4]),
            texture(6, &v[5]),
            texture(7, &v[6]),
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::Sampler(&self.clamp_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::Sampler(&self.repeat_sampler),
            },
            texture(10, &v[7]),
            texture(11, &v[8]),
            texture(12, albedo),
            texture(13, cover),
        ];
        if self.features {
            entries.push(texture(14, features));
        }
        if self.materials {
            entries.push(texture(15, materials));
        }
        if self.relief {
            entries.push(texture(16, relief.0));
            entries.push(texture(17, relief.1));
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain"),
            layout: &self.layout,
            entries: &entries,
        })
    }
}

/// The ground materials' array texture (`set_ground_materials`): a neutral 1 × 1 placeholder until materials are handed
/// in.
struct GroundTextures {
    view: wgpu::TextureView,
    /// Materials are handed in (not the placeholder), and shown (`show_ground_materials`).
    bound: bool,
    shown: bool,
    /// How strongly they show (`set_ground_material_strength`; [`MATERIAL_STRENGTH`] by default).
    strength: f32,
}

impl GroundTextures {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            view: ground_texture(device, queue, &crate::GroundMaterials::placeholder()),
            bound: false,
            shown: true,
            strength: MATERIAL_STRENGTH,
        }
    }
}

/// The ground materials' array, every level uploaded (whole blocks for BC7: a level under a block is one block).
fn ground_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    materials: &crate::GroundMaterials,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain ground materials"),
        size: wgpu::Extent3d {
            width: materials.side,
            height: materials.side,
            depth_or_array_layers: materials.layers,
        },
        mip_level_count: materials.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: materials.texels.format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (block, block_bytes) = match materials.texels {
        crate::ground_materials::MaterialTexels::Rgba8 => (1, 4),
        crate::ground_materials::MaterialTexels::Bc7 => (4, 16),
    };
    for (level, bytes) in materials.levels.iter().enumerate() {
        let side = materials.level_side(level);
        let blocks = side.div_ceil(block);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(blocks * block_bytes),
                rows_per_image: Some(blocks),
            },
            wgpu::Extent3d {
                width: blocks * block,
                height: blocks * block,
                depth_or_array_layers: materials.layers,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

/// The trees' placement's group 0: the view, the patches, the placed trees and the draw's arguments (`buffers`, in
/// that order), and the canopy's pages.
fn place_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 4],
    canopy: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let mut entries: Vec<wgpu::BindGroupEntry> = buffers
        .iter()
        .enumerate()
        .map(|(k, buffer)| wgpu::BindGroupEntry {
            binding: k as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    entries.push(wgpu::BindGroupEntry {
        binding: 4,
        resource: wgpu::BindingResource::TextureView(canopy),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("terrain tree placement"),
        layout,
        entries: &entries,
    })
}

/// The trees: their placement pass and its buffers, and the card draw.
struct TreeDraw {
    /// The impostors' depth prepass and their light (`near_trees::tree_pipelines`).
    pipelines: [PipelineId; 2],
    place: PipelineId,
    /// The ground cover's placement (`tree_place.wgsl`'s `place_cover`), same bindings.
    cover: PipelineId,
    /// The placement's group 0: the view, the patches, the placed trees, the draw's arguments and the canopy's pages;
    /// its layout, to make it again when the canopy's texture changes.
    place_group: wgpu::BindGroup,
    place_layout: wgpu::BindGroupLayout,
    view: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// The placed trees ([`TREE_CAPACITY`] × [`TREE_INSTANCE_BYTES`]), written by the placement, read as instances.
    trees: wgpu::Buffer,
    /// `DrawIndexedIndirectArgs`: the card's indices and the trees the placement counted.
    args: wgpu::Buffer,
    /// The placed trees into the sun's near and wide maps (`tree_caster.wgsl`).
    caster: PipelineId,
    /// The forest's shade (`tree_shade.wgsl`, into the open sky's green and blue) and the rectangles waiting for it
    /// (`forest_shade`).
    shade: crate::forest_shade::ForestShade,
    /// The trees' impostor atlas (a host's shared `TreeAtlas`, or the land's own) and its bake, if this land is to bake
    /// it.
    impostors: crate::impostors::Impostors,
    /// The trees near the camera as meshes (`near_trees`).
    near: crate::near_trees::NearTrees,
    /// The grass about the camera (`grass`).
    grass: crate::grass::Grass,
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX
            | wgpu::ShaderStages::FRAGMENT
            | wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn texture_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

/// The field's heights, in quanta (`R16Sint`), read texel by texel.
fn heights_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Sint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    )
}

/// The layout of naval-clouds' `CloudShadow` bind group (group-equivalent to `CloudShadow::bind_group_layout`).
pub fn cloud_shadow_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("terrain cloud shadow"),
        entries: &[
            texture_entry(0, true),
            entry(
                1,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            entry(
                2,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            ),
        ],
    })
}

fn view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Registers this crate's WGSL (and naval-sky's environment module and naval-clouds' shadow module, unless another layer
/// has) with the renderer.
pub(crate) fn register_shaders(ctx: &mut RenderContext<'_>) -> Result<(), ShaderError> {
    // With their disk paths, so they hot-reload.
    for file in [
        naval_sky::shaders::ENVIRONMENT_FILE,
        naval_clouds::shaders::SHADOW_FILE,
    ] {
        if ctx.library.text(file.path).is_none() {
            ctx.add_shader(file)?;
        }
    }
    for file in [
        shader_file!("shaders/bindings.wgsl"),
        shader_file!("shaders/hex.wgsl"),
        shader_file!("shaders/features.wgsl"),
        shader_file!("shaders/night.wgsl"),
        shader_file!("shaders/lamps.wgsl"),
        shader_file!("shaders/materials.wgsl"),
        shader_file!("shaders/surface.wgsl"),
        shader_file!("shaders/lighting.wgsl"),
        shader_file!("shaders/structures.wgsl"),
        shader_file!("shaders/cells.wgsl"),
        shader_file!("shaders/terrain.wgsl"),
        shader_file!("shaders/caster.wgsl"),
        shader_file!("shaders/sky_bake.wgsl"),
        shader_file!("shaders/foliage.wgsl"),
        shader_file!("shaders/tree_wind.wgsl"),
        shader_file!("shaders/trees.wgsl"),
        shader_file!("shaders/near_trees.wgsl"),
        shader_file!("shaders/tree_near_bin.wgsl"),
        shader_file!("shaders/tree_place.wgsl"),
        shader_file!("shaders/forest.wgsl"),
        shader_file!("shaders/tree_caster.wgsl"),
        shader_file!("shaders/tree_shade.wgsl"),
        shader_file!("shaders/impostor.wgsl"),
        shader_file!("shaders/tree_surface.wgsl"),
        shader_file!("shaders/tree_bake.wgsl"),
        shader_file!("shaders/tree_resolve.wgsl"),
        shader_file!("shaders/tree_mips.wgsl"),
        shader_file!("shaders/grass_place.wgsl"),
        shader_file!("shaders/grass.wgsl"),
    ] {
        ctx.add_shader(file)?;
    }
    Ok(())
}

/// The shader defs a map's land compiles with: its style and the detailed surface. `rock` (the cool temperate band a
/// theatre coast baked at 45-58 degrees takes: the Channel, the North Sea, Biscay) shades as the temperate `chalk`
/// country (downland, farmland between hedgerows, woods), where the web shaded it as `tropical` (rainforest and palm
/// rows off Brittany). No battle map with land is `rock`.
fn surface_defs(
    style: LandStyle,
    quality: &TerrainQuality,
    features: bool,
    materials: bool,
    relief: bool,
) -> naval_render::ShaderDefs {
    let name = match style {
        LandStyle::Tropical => "STYLE_TROPICAL",
        LandStyle::Snow => "STYLE_SNOW",
        LandStyle::Volcanic => "STYLE_VOLCANIC",
        LandStyle::Chalk | LandStyle::Rock => "STYLE_CHALK",
    };
    let mut defs = vec![(name.to_owned(), ShaderDefValue::Bool(true))];
    if quality.detailed {
        defs.push(("DETAILED".to_owned(), ShaderDefValue::Bool(true)));
    }
    if quality.trees {
        // The forest's shade under the trees (`forest.wgsl`); Medium, with no trees, draws the land as before.
        defs.push(("TREES".to_owned(), ShaderDefValue::Bool(true)));
    }
    if features {
        // Map features bound at 14 (`features.wgsl`).
        defs.push(("FEATURES".to_owned(), ShaderDefValue::Bool(true)));
    }
    if materials {
        // The ground materials bound at 15 (`materials.wgsl`).
        defs.push(("MATERIALS".to_owned(), ShaderDefValue::Bool(true)));
    }
    if relief {
        // The fine relief bound at 16 (`bindings.wgsl`'s `relief_height`).
        defs.push(relief_def());
    }
    defs
}

fn relief_def() -> (String, ShaderDefValue) {
    ("RELIEF".to_owned(), ShaderDefValue::Bool(true))
}

/// A patch's mesh of `quads`² quads, its vertices `unit` apart (in the plain patch's quads), every diagonal from (i, j)
/// to (i + 1, j + 1).
fn patch_mesh(quads: u32, unit: f32) -> (Vec<f32>, Vec<u16>) {
    let n = quads;
    let mut vertices = Vec::with_capacity(((n + 1) * (n + 1) * 2) as usize);
    for j in 0..=n {
        for i in 0..=n {
            vertices.extend_from_slice(&[i as f32 * unit, j as f32 * unit]);
        }
    }
    let mut indices: Vec<u16> = Vec::with_capacity((n * n * 6) as usize);
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u16;
            let (b, c) = (a + 1, a + n as u16 + 1);
            let d = c + 1;
            indices.extend_from_slice(&[a, c, d, a, d, b]);
        }
    }
    (vertices, indices)
}

/// Most level-0 patches drawn dense at once (`lod::DENSE_LAND`).
const DENSE_CAPACITY: usize = 1024;

/// The dense patches' mesh and this view's copies of them (`lod::DENSE_COPY`).
struct DensePatches {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    count: usize,
}

/// Frustum planes of a (reversed-depth) view-projection, inside where n · p + c ≥ 0 (three's
/// `Frustum.setFromProjectionMatrix`), normalised.
pub fn frustum_planes(view_projection: Mat4) -> Vec<[f64; 4]> {
    let rows = [
        view_projection.row(0),
        view_projection.row(1),
        view_projection.row(2),
        view_projection.row(3),
    ];
    let planes: [Vec4; 6] = [
        rows[3] + rows[0],
        rows[3] - rows[0],
        rows[3] + rows[1],
        rows[3] - rows[1],
        // Reversed depth in [0, 1]: z ≤ w at the near plane, z ≥ 0 at the far one.
        rows[3] - rows[2],
        rows[2],
    ];
    planes
        .iter()
        .map(|p| {
            let length = p.truncate().length().max(1e-12);
            let p = p.as_dvec4() / f64::from(length);
            [p.x, p.y, p.z, p.w]
        })
        .collect()
}

/// The trees' parts in the scene, each shown or hidden for measurements (`TerrainLayer::show_tree_part`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreePart {
    /// The near meshes' full detail.
    NearFull,
    /// The near meshes' reduced detail.
    NearReduced,
    /// The impostors (their caster cards in the sun's maps stay: `set_tree_shadows`).
    Impostors,
    /// The grass.
    Grass,
}

impl TreePart {
    /// By name: `near-full`, `near-reduced`, `impostors`, `grass`.
    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "near-full" => Some(Self::NearFull),
            "near-reduced" => Some(Self::NearReduced),
            "impostors" => Some(Self::Impostors),
            "grass" => Some(Self::Grass),
            _ => None,
        }
    }
}

impl TerrainLayer {
    /// Prepares `field` for `scene` (its style, fog and where the chart stands) and builds the layer's GPU
    /// resources and pipelines. `environment` is the sky's environment bind group
    /// (`naval_sky::EnvironmentMap::bind_group`).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        field: &Heightfield,
        scene: &MapScene,
        settings: TerrainSettings,
        environment: &wgpu::BindGroup,
    ) -> Result<Self, ShaderError> {
        Self::with_prepared(
            ctx,
            PreparedField::new(field),
            scene.style,
            scene.offset,
            scene.fog,
            settings,
            environment,
        )
    }

    /// As [`TerrainLayer::new`] from an already prepared field.
    pub fn with_prepared(
        ctx: &mut RenderContext<'_>,
        prepared: PreparedField,
        style: SurfaceStyle,
        offset: [f64; 2],
        fog: FogParameters,
        settings: TerrainSettings,
        environment: &wgpu::BindGroup,
    ) -> Result<Self, ShaderError> {
        Self::from_land(
            ctx,
            PreparedLand::from_prepared(prepared),
            style,
            offset,
            fog,
            settings,
            environment,
        )
    }

    /// As [`TerrainLayer::new`] from the land's CPU half ([`PreparedLand`], prepared on another thread): only the
    /// uploads, the bind groups and the pipelines are left for the render thread.
    pub fn for_scene(
        ctx: &mut RenderContext<'_>,
        land: PreparedLand,
        scene: &MapScene,
        settings: TerrainSettings,
        environment: &wgpu::BindGroup,
    ) -> Result<Self, ShaderError> {
        Self::for_scene_sharing(ctx, land, scene, settings, environment, None)
    }

    /// As [`TerrainLayer::for_scene`], its trees drawn from `atlas` (a host's, made on this device, shared by every land
    /// it builds: `impostors`), or from an atlas of its own when none.
    pub fn for_scene_sharing(
        ctx: &mut RenderContext<'_>,
        land: PreparedLand,
        scene: &MapScene,
        settings: TerrainSettings,
        environment: &wgpu::BindGroup,
        atlas: Option<&crate::impostors::TreeAtlas>,
    ) -> Result<Self, ShaderError> {
        Self::build(
            ctx,
            land,
            scene.style,
            scene.offset,
            scene.fog,
            settings,
            environment,
            atlas,
        )
    }

    /// The layer from the land's CPU half, placed and coloured as [`TerrainLayer::with_prepared`] takes them.
    pub fn from_land(
        ctx: &mut RenderContext<'_>,
        land: PreparedLand,
        style: SurfaceStyle,
        offset: [f64; 2],
        fog: FogParameters,
        settings: TerrainSettings,
        environment: &wgpu::BindGroup,
    ) -> Result<Self, ShaderError> {
        Self::build(ctx, land, style, offset, fog, settings, environment, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        ctx: &mut RenderContext<'_>,
        land: PreparedLand,
        style: SurfaceStyle,
        offset: [f64; 2],
        fog: FogParameters,
        settings: TerrainSettings,
        environment: &wgpu::BindGroup,
        atlas: Option<&crate::impostors::TreeAtlas>,
    ) -> Result<Self, ShaderError> {
        let PreparedLand {
            field: mut prepared,
            gradient_levels,
            attribute_levels,
            noise_levels,
            detail_levels,
            sunlight,
            ground_materials,
        } = land;
        register_shaders(ctx)?;
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        let grid = prepared.grid;
        let (columns, rows) = (grid.columns, grid.rows);
        let chart = TerrainChart {
            origin_x: f64::from(prepared.origin[0]),
            origin_z: f64::from(prepared.origin[1]),
            width: f64::from(columns - 1) * grid.cell,
            depth: f64::from(rows - 1) * grid.cell,
            bounds: prepared.bounds.clone(),
            cell: grid.cell,
        };
        let mut lod =
            TerrainLod::new(chart, settings.quality.lod).map_err(|message| ShaderError {
                path: "naval-terrain".into(),
                message,
            })?;
        // Every level-0 patch with land carries trees (and the level-1 ones, `select`): their placement thins them with
        // distance (the web's 256 nearest patches are `TerrainLod`'s default, which its pins hold).
        lod.tree_patches = PATCH_CAPACITY;

        // Textures.
        // The samples as stored, in quanta (`terrain.params.z` metres each): half R32Float's memory, the same heights.
        let heights = texture_with_levels(
            &device,
            &queue,
            "terrain heights",
            wgpu::TextureFormat::R16Sint,
            columns,
            rows,
            &[bytemuck::cast_slice(&prepared.quanta)],
            wgpu::TextureUsages::empty(),
        );
        let gradient_bytes: Vec<&[u8]> = gradient_levels
            .iter()
            .map(|l| bytemuck::cast_slice(l))
            .collect();
        let gradients = texture_with_levels(
            &device,
            &queue,
            "terrain gradients",
            wgpu::TextureFormat::Rg16Float,
            columns,
            rows,
            &gradient_bytes,
            wgpu::TextureUsages::empty(),
        );
        let coarse = prepared.coarse;
        let attribute_bytes: Vec<&[u8]> = attribute_levels.iter().map(Vec::as_slice).collect();
        let attributes = texture_with_levels(
            &device,
            &queue,
            "terrain attributes",
            wgpu::TextureFormat::Rgba8Unorm,
            coarse.columns,
            coarse.rows,
            &attribute_bytes,
            wgpu::TextureUsages::empty(),
        );
        // The land's own shadow when the CPU half computed it for the scene's light; else lit everywhere until the
        // first frame computes it (`Sunlight::follow`).
        let (sun, sunlight_bytes) = match sunlight {
            Some((direction, bytes)) if bytes.len() == (columns * rows) as usize => {
                (Some(direction), bytes)
            }
            _ => (None, vec![255u8; (columns * rows) as usize]),
        };
        let sunlight_texture = texture_with_levels(
            &device,
            &queue,
            "terrain sunlight",
            wgpu::TextureFormat::R8Unorm,
            columns,
            rows,
            &[&sunlight_bytes],
            // Copied out only by tests and probes (`sunlight_texture`).
            wgpu::TextureUsages::COPY_SRC,
        );
        let open_sky = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain open sky"),
            size: wgpu::Extent3d {
                width: coarse.columns,
                height: coarse.rows,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OPEN_SKY_FORMAT,
            // Copied out only by tests and probes (`open_sky_texture`); copied into with the forest's shade.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let noise_bytes: Vec<&[u8]> = noise_levels.iter().map(Vec::as_slice).collect();
        let noise_texture = texture_with_levels(
            &device,
            &queue,
            "terrain noise",
            wgpu::TextureFormat::Rgba8Unorm,
            NOISE_SIZE as u32,
            NOISE_SIZE as u32,
            &noise_bytes,
            wgpu::TextureUsages::empty(),
        );
        // Neutral (every channel at its mean) until the photographs are handed in, unless the CPU half packed them.
        let detail_bytes: Vec<&[u8]> = detail_levels.iter().map(Vec::as_slice).collect();
        let detail_texture = texture_with_levels(
            &device,
            &queue,
            "terrain detail",
            wgpu::TextureFormat::Rgba8Unorm,
            DETAIL_SIZE,
            DETAIL_SIZE,
            &detail_bytes,
            wgpu::TextureUsages::empty(),
        );
        let dfg = texture_with_levels(
            &device,
            &queue,
            "terrain DFG",
            wgpu::TextureFormat::Rg16Float,
            DFG_SIZE,
            DFG_SIZE,
            &[bytemuck::cast_slice(&DFG_LUT)],
            wgpu::TextureUsages::empty(),
        );
        // The trees' impostors, baked on the GPU in the first frame unless a land sharing the atlas has (`impostors`); a
        // placeholder without trees.
        let mut impostors = if settings.quality.trees {
            Some(crate::impostors::Impostors::new(ctx, atlas)?)
        } else {
            None
        };
        let atlas_texture = impostors.as_ref().map_or_else(
            || crate::impostors::placeholder(&device),
            |i| i.texture.clone(),
        );
        let sampler = |label: &str, mode: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: 8,
                ..Default::default()
            })
        };
        let clamp_sampler = sampler("terrain clamp", wgpu::AddressMode::ClampToEdge);
        let repeat_sampler = sampler("terrain repeat", wgpu::AddressMode::Repeat);

        // Layouts and bind groups. Map features (binding 14) only on a device allowing 17 sampled textures a stage: the
        // land's fragment stage samples 16 without them, WebGPU's default (the client's devices ask for 24).
        let sampled = device.limits().max_sampled_textures_per_shader_stage;
        let features_supported = sampled >= 17;
        // The ground materials (binding 15) on the High setting, on a device allowing one more texture a stage than the
        // land takes without them (16, and the features' one where it binds them).
        let materials_supported =
            settings.quality.materials && sampled >= 17 + u32::from(features_supported);
        // The fine relief (bindings 16 and 17) on the High setting, on a device allowing two more textures a stage again.
        let relief_supported = settings.quality.relief
            && sampled >= 18 + u32::from(features_supported) + u32::from(materials_supported);
        let mut terrain_entries = vec![
            entry(
                0,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            ),
            heights_entry(1),
            texture_entry(2, true),
            texture_entry(3, true),
            texture_entry(4, true),
            texture_entry(5, true),
            texture_entry(6, true),
            texture_entry(7, true),
            entry(
                8,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            entry(
                9,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            texture_entry(10, true),
            // The trees' impostors: a 2D array, colour layers then normal layers (`impostors`).
            entry(
                11,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
            ),
            // The surface maps: satellite colour (filtered) and land cover (class bytes, read texel by texel), the
            // window's maps in layer 0 and the near tiles in layer 1 (`surface_textures`).
            entry(
                12,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
            ),
            entry(
                13,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
            ),
        ];
        if features_supported {
            // Map features (`features`): each level's distances and attributes, filtered, read by the fragment stage and
            // the trees' placement only (the vertex stage stays within the default 16 textures).
            terrain_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 14,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            });
        }
        if materials_supported {
            // The ground materials (`ground_materials`): two layers a material, filtered, for the land's fragment stage
            // and the grass's placement (which colours the grass as the land's surface is coloured).
            terrain_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 15,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            });
        }
        if relief_supported {
            // The fine relief (`relief`): residual heights a layer a page, filtered, read wherever the land's height is.
            terrain_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 16,
                visibility: wgpu::ShaderStages::VERTEX
                    | wgpu::ShaderStages::FRAGMENT
                    | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            });
            // Their slopes, for the land's shading (and the grass's placement, which takes the land's surface).
            terrain_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 17,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            });
        }
        let terrain_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain"),
            entries: &terrain_entries,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain uniforms"),
            size: std::mem::size_of::<TerrainUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let views = [
            view(&heights),
            view(&gradients),
            view(&attributes),
            view(&sunlight_texture),
            view(&open_sky),
            view(&noise_texture),
            view(&detail_texture),
            view(&dfg),
            crate::impostors::atlas_view(&atlas_texture),
        ];
        // No surface maps until a host hands some in (`set_surface_maps`): an empty colour and an unknown cover.
        let surface = crate::surface_textures::SurfaceTextures::new(&device);
        let features = crate::features::FeatureTextures::new(&device);
        let canopy = crate::canopy::CanopyTextures::new(&device);
        let relief = crate::relief::ReliefTextures::new(&device);
        let mut ground = GroundTextures::new(&device, &queue);
        if materials_supported
            && let Some(materials) = &ground_materials
            && materials.is_consistent()
            && materials.layers >= crate::GroundMaterials::LAYERS
            && (materials.texels != crate::ground_materials::MaterialTexels::Bc7
                || device
                    .features()
                    .contains(wgpu::Features::TEXTURE_COMPRESSION_BC))
        {
            ground.view = ground_texture(&device, &queue, materials);
            ground.bound = true;
        }
        let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain open sky bake"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        let bake_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain open sky bake"),
            layout: &bake_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&views[4]),
            }],
        });
        let group_parts = GroupParts {
            layout: terrain_layout.clone(),
            uniforms: uniform_buffer.clone(),
            views,
            clamp_sampler,
            repeat_sampler,
            features: features_supported,
            materials: materials_supported,
            relief: relief_supported,
        };
        let terrain_group = {
            let (albedo, cover) = surface.views();
            group_parts.bind_group(
                &device,
                albedo,
                cover,
                features.view(),
                &ground.view,
                relief.views(),
            )
        };
        // The bake's inputs alone: the open sky it writes is sampled by the terrain group.
        let bake_input_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain open sky bake inputs"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                heights_entry(1),
                texture_entry(2, true),
            ],
        });
        let bake_inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain open sky bake inputs"),
            layout: &bake_input_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&group_parts.views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&group_parts.views[1]),
                },
            ],
        });
        let environment_layout = naval_sky::environment_bind_group_layout(&device);
        let cloud_layout = cloud_shadow_layout(&device);
        let placeholder_map = texture_with_levels(
            &device,
            &queue,
            "terrain cloud shadow placeholder",
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            1,
            &[&[255, 255, 255, 255]],
            wgpu::TextureUsages::empty(),
        );
        let placeholder_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain cloud shadow placeholder"),
            // naval-clouds' `CloudShadowUniforms` (64 bytes; `naval_clouds::shadow`'s `CloudShadow`).
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let placeholder_view = view(&placeholder_map);
        let cloud_placeholder = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain cloud shadow placeholder"),
            layout: &cloud_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&placeholder_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&group_parts.repeat_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: placeholder_uniforms.as_entire_binding(),
                },
            ],
        });

        // Geometry: one patch of (PATCH_QUADS + 1)² vertices, every diagonal from (i, j) to (i + 1, j + 1).
        let (vertices, indices) = patch_mesh(PATCH_QUADS, 1.0);
        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: contents.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, contents);
            buffer
        };
        let grid_vertices = buffer(
            "terrain patch vertices",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let grid_indices = buffer(
            "terrain patch indices",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        );
        // The dense patch (`lod::DENSE_LAND`): twice the vertices a side, in halves of the plain patch's.
        let (dense_vertices, dense_indices) = patch_mesh(2 * PATCH_QUADS, 0.5);
        let dense = DensePatches {
            vertices: buffer(
                "terrain dense patch vertices",
                bytemuck::cast_slice(&dense_vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: buffer(
                "terrain dense patch indices",
                bytemuck::cast_slice(&dense_indices),
                wgpu::BufferUsages::INDEX,
            ),
            instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain dense patches"),
                size: (DENSE_CAPACITY * PATCH_FLOATS * 4) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            count: 0,
        };
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain patches"),
            size: (PATCH_CAPACITY * PATCH_FLOATS * 4) as u64,
            // Read by the trees' placement too.
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let caster_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain caster patches"),
            size: (CASTER_CAPACITY * PATCH_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Pipelines.
        let patch_buffers = vec![
            VertexLayout {
                stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: vec![wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                }],
            },
            VertexLayout {
                stride: (PATCH_FLOATS * 4) as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: vec![wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 1,
                }],
            },
        ];
        let defs = surface_defs(
            style.style,
            &settings.quality,
            group_parts.features,
            group_parts.materials,
            group_parts.relief,
        );
        let land = ctx.add_scene_pipeline(
            "terrain land",
            "naval-terrain/shaders/terrain.wgsl",
            ScenePipeline {
                vertex_buffers: patch_buffers.clone(),
                bind_group_layouts: vec![
                    terrain_layout.clone(),
                    environment_layout.clone(),
                    cloud_layout.clone(),
                ],
                defs: defs.clone(),
                ..ScenePipeline::new("vs_land", "fs_land")
            },
        )?;
        let caster = ctx.add_caster_pipeline(
            "terrain caster",
            "naval-terrain/shaders/caster.wgsl",
            CasterPipeline {
                vertex_buffers: patch_buffers,
                bind_group_layouts: vec![terrain_layout.clone()],
                // The land casts as it is drawn, with its fine relief.
                defs: if group_parts.relief {
                    vec![relief_def()]
                } else {
                    Vec::new()
                },
                ..CasterPipeline::new("vs_caster")
            },
        )?;
        let bake = ctx.add_pipeline(
            "terrain open sky bake",
            "naval-terrain/shaders/sky_bake.wgsl",
            Vec::new(),
            compute_recipe(vec![bake_layout, bake_input_layout], "bake"),
        )?;
        let lamps = if settings.quality.detailed {
            let top = prepared.top();
            Some(crate::night::NightLamps::new(
                ctx,
                &terrain_layout,
                defs.clone(),
                top,
            )?)
        } else {
            None
        };
        let trees = if settings.quality.trees {
            let (card_vertices, card_indices) = tree_card();
            let mut tree_defs = defs;
            tree_defs.push(("LOD_ZERO".to_owned(), ShaderDefValue::Bool(true)));
            let float4 = |location: u32, offset: u64| wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset,
                shader_location: location,
            };
            // The card's corners, and the placed trees as instances.
            let tree_buffers = vec![
                VertexLayout {
                    stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: vec![wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                },
                VertexLayout {
                    stride: TREE_INSTANCE_BYTES,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: vec![float4(1, 0), float4(2, 16), float4(3, 32)],
                },
            ];
            let pipelines = crate::near_trees::tree_pipelines(
                ctx,
                "terrain trees",
                "naval-terrain/shaders/trees.wgsl",
                ScenePipeline {
                    vertex_buffers: tree_buffers.clone(),
                    bind_group_layouts: vec![
                        terrain_layout.clone(),
                        environment_layout.clone(),
                        cloud_layout.clone(),
                    ],
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    defs: tree_defs.clone(),
                    ..ScenePipeline::new("vs_tree", "fs_tree")
                },
                "fs_tree_depth",
            )?;
            let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
            let place_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain tree placement"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    storage(1, true),
                    storage(2, false),
                    storage(3, false),
                    // The canopy height map's pages (`canopy`), read texel by texel.
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
            let place = ctx.add_pipeline(
                "terrain tree placement",
                "naval-terrain/shaders/tree_place.wgsl",
                tree_defs.clone(),
                compute_recipe(vec![place_layout.clone(), terrain_layout.clone()], "place"),
            )?;
            let cover = ctx.add_pipeline(
                "terrain ground cover placement",
                "naval-terrain/shaders/tree_place.wgsl",
                tree_defs.clone(),
                compute_recipe(
                    vec![place_layout.clone(), terrain_layout.clone()],
                    "place_cover",
                ),
            )?;
            // The trees into the sun's maps: the same instances, a card facing the light, alpha-tested.
            let caster = ctx.add_caster_pipeline(
                "terrain tree caster",
                "naval-terrain/shaders/tree_caster.wgsl",
                CasterPipeline {
                    vertex_buffers: tree_buffers,
                    bind_group_layouts: vec![terrain_layout.clone()],
                    fragment_entry: Some("fs_tree_caster"),
                    ..CasterPipeline::new("vs_tree_caster")
                },
            )?;
            // Baked whole on the first frame, after the open sky.
            let mut shade = crate::forest_shade::ForestShade::new(
                ctx,
                &terrain_layout,
                tree_defs.clone(),
                OPEN_SKY_FORMAT,
            )?;
            shade.mark_all([coarse.columns, coarse.rows]);
            let view = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain tree view"),
                size: std::mem::size_of::<TreeView>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let placed = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain trees placed"),
                size: u64::from(TREE_CAPACITY) * TREE_INSTANCE_BYTES,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            // The card's six indices from its first; the instance count is the placement's.
            let args = buffer(
                "terrain tree draw",
                bytemuck::cast_slice(&[TREE_CARD_INDICES, 0, 0, 0, 0]),
                wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC,
            );
            let place_group = place_group(
                &device,
                &place_layout,
                [&view, &instances, &placed, &args],
                canopy.view(),
            );
            let grass = crate::grass::Grass::new(
                ctx,
                &terrain_layout,
                vec![
                    terrain_layout.clone(),
                    environment_layout.clone(),
                    cloud_layout.clone(),
                ],
                tree_defs.clone(),
            )?;
            let near = crate::near_trees::NearTrees::new(
                ctx,
                &placed,
                &args,
                vec![
                    terrain_layout.clone(),
                    environment_layout,
                    cloud_layout.clone(),
                ],
                tree_defs,
            )?;
            Some(TreeDraw {
                pipelines,
                place,
                cover,
                place_group,
                place_layout,
                view,
                vertices: buffer(
                    "tree card",
                    bytemuck::cast_slice(&card_vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buffer(
                    "tree card indices",
                    bytemuck::cast_slice(&card_indices),
                    wgpu::BufferUsages::INDEX,
                ),
                trees: placed,
                args,
                caster,
                shade,
                impostors: impostors.take().expect("the impostors with trees"),
                near,
                grass,
            })
        } else {
            None
        };

        let mut layer = Self {
            settings,
            style,
            offset,
            fog,
            lod,
            patches: vec![0f32; PATCH_CAPACITY * PATCH_FLOATS],
            patch_count: 0,
            tree_patches: 0,
            tree_range: 0.0,
            tree_view: TreeView {
                species: species_table(),
                ..TreeView::zeroed()
            },
            region_key: None,
            cliff_key: None,
            rock_faces: true,
            trees_stale: true,
            grass_view: crate::grass::GrassView::default(),
            grass_stale: true,
            country: None,
            tree_shadows: true,
            tree_casts: Default::default(),
            trees_cast_land: false,
            tree_planes: [[0.0; 4]; 6],
            tree_margins: [0.0; 4],
            tree_shadow_patches: 0,
            near_trees: true,
            hidden_parts: 0,
            near_view: crate::near_trees::NearView::default(),
            near_eye: [0.0; 3],
            wind: [1.0, 0.0, 0.0],
            wind_clock: 0.0,
            last_view: None,
            uniforms: TerrainUniforms::zeroed(),
            uniform_buffer,
            grid_vertices,
            grid_indices,
            instances,
            dense,
            caster_instances,
            caster_ranges: Default::default(),
            sunlight: Sunlight::new(sunlight_texture, sun, sunlight_bytes),
            detail_texture,
            terrain_group,
            group_parts,
            surface,
            surface_version: 0,
            cover_marks: 0,
            features,
            canopy,
            relief,
            ground,
            ground_held: ground_materials.clone(),
            camera_chart: None,
            bake_group,
            bake_inputs,
            environment: environment.clone(),
            cloud_layout,
            cloud_group: cloud_placeholder.clone(),
            cloud_placeholder,
            cloud_bound: false,
            land,
            caster,
            bake,
            baked: false,
            open_sky,
            trees,
            lamps,
            night_lights: Default::default(),
            night: 0.0,
            night_shown: true,
            prepared: Arc::new({
                prepared.release_uploaded();
                prepared
            }),
        };
        layer.set_light(&TerrainLight {
            light: CelestialLight {
                direction: Vec3::new(0.0, 1.0, 0.0),
                color: Vec3::ONE,
                intensity: 5.0,
                night: false,
            },
            ambient: 0.65,
            flash: 0.0,
        });
        Ok(layer)
    }

    /// How the land draws.
    pub fn settings(&self) -> TerrainSettings {
        self.settings
    }

    /// Draw one of the debug views instead of the lit land (`TerrainDebug::Off` for the land).
    pub fn set_debug(&mut self, debug: TerrainDebug) {
        self.settings.debug = debug;
    }

    /// Draw the land into the sun's near and wide shadow maps, and the view maps too when `view_maps`, or stop
    /// (`TerrainSettings::cast_shadows`).
    pub fn set_cast_shadows(&mut self, cast: bool, view_maps: bool) {
        self.settings.cast_shadows = cast;
        self.settings.cast_view_maps = view_maps;
    }

    /// The land's patches drawn into each of the sun's shadow maps in the last frame (near, wide, view 1, view 2,
    /// land 1, land 2, land 3); a patch is `PATCH_QUADS`² quads.
    pub fn land_shadow_patches(&self) -> [usize; naval_render::shadows::MAP_COUNT] {
        self.caster_ranges.map(|(_, count)| count as usize)
    }

    /// Whether the trees cast into the sun's near, wide and aboard maps while those are on (the default), with the forest's
    /// shade standing in beyond them; off, the forest's shade stands in everywhere (native/docs/terrain/trees.md).
    pub fn set_tree_shadows(&mut self, cast: bool) {
        self.tree_shadows = cast;
    }

    /// The maps the trees cast into in the last frame (near, wide, aboard), for tests and probes.
    pub fn tree_shadow_maps(&self) -> [bool; 3] {
        self.tree_casts.casts
    }

    /// Shows or hides a place's rock faces (native/docs/terrain/ground.md, "Cliffs"): hidden, its land draws as with no
    /// place for them, the photograph fading on the slopes to the style's rock (for comparisons and measurements).
    pub fn show_rock_faces(&mut self, shown: bool) {
        self.rock_faces = shown;
    }

    /// Whether the trees near the camera are drawn as meshes (the default with trees; native/docs/terrain/trees.md,
    /// "Near trees"); off, the impostors draw every tree.
    pub fn set_near_trees(&mut self, on: bool) {
        if self.near_trees != on {
            self.near_trees = on;
            self.trees_stale = true;
        }
    }

    /// Shows or hides one of the trees' parts in the scene, all shown by default: for measuring what each costs (the
    /// stills example's `--part-cost`). A hidden part leaves its pixels to what stands behind it; nothing else changes.
    pub fn show_tree_part(&mut self, part: TreePart, shown: bool) {
        let bit = 1 << part as u8;
        if shown {
            self.hidden_parts &= !bit;
        } else {
            self.hidden_parts |= bit;
        }
    }

    fn shows(&self, part: TreePart) -> bool {
        self.hidden_parts & (1 << part as u8) == 0
    }

    /// The near trees' indirect draws (a species' full detail, then its reduced, each 20 bytes) and their lists, for
    /// tests.
    pub fn near_tree_buffers(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer)> {
        self.trees
            .as_ref()
            .map(|t| (t.near.arguments(), t.near.lists()))
    }

    /// The wind the trees sway in: the true wind (world m/s, the way it blows; `naval_client`'s `ships.wind`). A calm
    /// (under 0.4 m/s) leaves them still.
    pub fn set_wind(&mut self, wind: Vec3) {
        let level = glam::Vec2::new(wind.x, wind.z);
        let speed = level.length();
        let way = if speed > 1e-4 {
            level / speed
        } else {
            glam::Vec2::X
        };
        self.wind = [way.x, way.y, speed];
    }

    /// Whether the trees cast into the sun's land maps in the last frame.
    pub fn trees_cast_land(&self) -> bool {
        self.trees_cast_land
    }

    /// The land about `eye` (world metres) within `radius` across the ground, for the sun's land maps
    /// ([`naval_render::Sun::land`]): its highest point and how far the nearest land above the sea lies; `None` with
    /// none. Read from the bound pyramid at a level some sixteen blocks across the radius, so it errs high and near.
    pub fn land_about(&self, eye: Vec3, radius: f32) -> Option<naval_render::LandView> {
        let bounds = &self.prepared.bounds;
        if bounds.levels.is_empty() || radius <= 0.0 {
            return None;
        }
        let chart = &self.lod.chart;
        let base = bounds.base as f64 * chart.cell;
        let wanted = f64::from(radius) / 16.0;
        let level = (0..bounds.levels.len())
            .take_while(|&l| l == 0 || base * f64::from(1u32 << l) <= wanted)
            .last()
            .unwrap_or(0);
        let blocks = &bounds.levels[level];
        let block = base * f64::from(1u32 << level);
        let (x, z) = (
            f64::from(eye.x) - self.offset[0] - chart.origin_x,
            f64::from(eye.z) - self.offset[1] - chart.origin_z,
        );
        let r = f64::from(radius);
        let span = |v: f64| {
            let first = ((v - r) / block).floor().max(0.0) as usize;
            let last = (((v + r) / block).floor().max(-1.0) + 1.0) as usize;
            first..last.min(blocks.size)
        };
        let mut found: Option<naval_render::LandView> = None;
        for j in span(z) {
            for i in span(x) {
                let high = blocks.max[j * blocks.size + i];
                if high <= LAND_ABOVE_SEA_M {
                    continue;
                }
                let dx = (x - x.clamp(i as f64 * block, (i + 1) as f64 * block)) as f32;
                let dz = (z - z.clamp(j as f64 * block, (j + 1) as f64 * block)) as f32;
                let distance = dx.hypot(dz);
                if distance > radius {
                    continue;
                }
                let view = found.get_or_insert(naval_render::LandView {
                    top: high,
                    nearest: distance,
                });
                view.top = view.top.max(high);
                view.nearest = view.nearest.min(distance);
            }
        }
        found
    }

    /// The prepared field this land draws: its samples, grids and bounds (the arrays it uploaded are let go).
    pub fn field(&self) -> &PreparedField {
        &self.prepared
    }

    /// The same field, shared: a host's height queries (the camera's clearance) read it rather than a copy.
    pub fn shared_field(&self) -> Arc<PreparedField> {
        self.prepared.clone()
    }

    /// The open sky the layer baked on its first frame (`Rgba8Unorm` on the attribute grid, the share in red), for
    /// tests and probes; [`crate::open_sky::open_sky`] is the same on the CPU.
    pub fn open_sky_texture(&self) -> &wgpu::Texture {
        &self.open_sky
    }

    /// World = chart + offset.
    pub fn offset(&self) -> [f64; 2] {
        self.offset
    }

    /// Moves the land to world = chart + `offset` from the next frame: a theatre cruise scene's land follows where the
    /// theatre puts it as its formation is drawn (`naval-client`'s `theatre::land`). The patches are chosen again.
    pub fn set_offset(&mut self, offset: [f64; 2]) {
        if offset != self.offset {
            self.offset = offset;
            self.last_view = None;
        }
    }

    /// A clearing from the next frame: no tree stands within `radius` metres of chart point `centre` (thinning back to
    /// the cover over `edge` more), and the land there is mown grass (a theatre airfield's apron, where its flight
    /// stands parked). `None` clears none.
    pub fn set_clearing(&mut self, clearing: Option<([f64; 2], f64, f64)>) {
        self.trees_stale = true;
        self.uniforms.clearing = match clearing {
            Some((centre, radius, edge)) => [
                centre[0] as f32,
                centre[1] as f32,
                radius as f32,
                edge.max(1.0) as f32,
            ],
            None => [0.0; 4],
        };
    }

    /// Clearings from the next frame, a group of capsules each (a generated base's, `infra::bases::Plan::clearing`):
    /// no tree stands in them (thinning back to the cover over `edge` metres, the edge ragged), and the land there is
    /// mown grass, as in [`Self::set_clearing`]'s. Groups past [`CLEARING_GROUPS`] and capsules past
    /// [`CLEARING_CAPSULES`] are left out (the host hands the nearest first). The trees are placed again only when the
    /// clearings change.
    pub fn set_clearings(&mut self, groups: &[ClearingGroup], edge: f32) {
        let mut head = [0.0f32; 4];
        let mut boxes = [[0.0f32; 4]; CLEARING_GROUPS];
        let mut ranges = [[0u32; 4]; CLEARING_GROUPS / 2];
        let mut capsules = [[0.0f32; 4]; CLEARING_CAPSULES];
        let mut radii = [[0.0f32; 4]; CLEARING_CAPSULES / 4];
        let mut used = 0usize;
        let mut held = 0usize;
        // How far a capsule's ragged edge reaches past its radius (`clearing_at`): its feather and wander.
        let reach = edge.max(1.0) * 3.2;
        for group in groups.iter().take(CLEARING_GROUPS) {
            let room = CLEARING_CAPSULES - used;
            let take = group.capsules.len().min(room);
            if take == 0 {
                continue;
            }
            // Its rectangle, and no more than its capsules reach.
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for (k, c) in group.capsules.iter().take(take).enumerate() {
                let i = used + k;
                capsules[i] = [c.a[0], c.a[1], c.b[0], c.b[1]];
                // A soft capsule's radius goes negative (`bindings.wgsl`'s `clearing_at`).
                radii[i / 4][i % 4] = if c.soft { -c.radius } else { c.radius };
                for p in [c.a, c.b] {
                    lo = [lo[0].min(p[0] - c.radius), lo[1].min(p[1] - c.radius)];
                    hi = [hi[0].max(p[0] + c.radius), hi[1].max(p[1] + c.radius)];
                }
            }
            let lo = [lo[0].max(group.min[0]), lo[1].max(group.min[1])];
            let hi = [hi[0].min(group.max[0]), hi[1].min(group.max[1])];
            boxes[held] = [lo[0] - reach, lo[1] - reach, hi[0] + reach, hi[1] + reach];
            let r = &mut ranges[held / 2];
            let at = (held % 2) * 2;
            r[at] = used as u32;
            // The count in the low 16 bits, the temperate flag in bit 16 (`bindings.wgsl`'s `clearing_at`).
            r[at + 1] = take as u32 | (u32::from(group.temperate) << 16);
            used += take;
            held += 1;
        }
        head[0] = held as f32;
        head[1] = edge.max(1.0);
        let changed = head != self.uniforms.clearing_head
            || boxes != self.uniforms.clearing_boxes
            || ranges != self.uniforms.clearing_ranges
            || capsules != self.uniforms.clearing_capsules
            || radii != self.uniforms.clearing_radii;
        if changed {
            self.uniforms.clearing_head = head;
            self.uniforms.clearing_boxes = boxes;
            self.uniforms.clearing_ranges = ranges;
            self.uniforms.clearing_capsules = capsules;
            self.uniforms.clearing_radii = radii;
            self.trees_stale = true;
        }
    }

    /// The clearings' groups and capsules held ([`Self::set_clearings`]).
    pub fn clearing_counts(&self) -> (usize, usize) {
        let groups = self.uniforms.clearing_head[0] as usize;
        let capsules = (0..groups)
            .map(|g| self.uniforms.clearing_ranges[g / 2][(g % 2) * 2 + 1] as usize)
            .sum();
        (groups, capsules)
    }

    /// Patches drawn in the last frame, and the patches with land among them that carry trees (level 0 and 1).
    pub fn patch_counts(&self) -> (usize, usize) {
        (self.patch_count, self.tree_patches)
    }

    /// The trees' impostor atlas (`impostors`: colour layers then normal layers), baked in the first frame; `None`
    /// without trees.
    pub fn tree_impostors(&self) -> Option<&wgpu::Texture> {
        self.trees.as_ref().map(|trees| &trees.impostors.texture)
    }

    /// Whether this land is to bake the device's impostor atlas (no land has yet), for tests and probes.
    pub fn bakes_tree_impostors(&self) -> bool {
        self.trees
            .as_ref()
            .is_some_and(|trees| trees.impostors.holds_bake())
    }

    /// The rectangles of the attribute grid waiting for the forest's shade (texels: first column and row, one past the
    /// last), for tests and probes.
    pub fn forest_shade_waiting(&self) -> &[crate::forest_shade::TexelRect] {
        self.trees
            .as_ref()
            .map_or(&[], |trees| trees.shade.waiting())
    }

    /// A thread to compute the land's own shadow on as the sun moves (`sunlight`; native/docs/terrain/shadows.md, "The
    /// land's own shadow"), or none: the render thread then computes it in `prepare`, as a layer made without one does.
    pub fn set_worker(&mut self, worker: Option<LandWorker>) {
        self.sunlight.set_worker(worker);
    }

    /// Whether the worker takes the land's shadow (the default), or the render thread computes it (a manual clock's
    /// scripted frames, which must repeat frame for frame).
    pub fn set_background_work(&mut self, background: bool) {
        self.sunlight.set_background(background);
    }

    /// Whether the land's own shadow is being computed on the worker (the old one drawn meanwhile).
    pub fn sun_pending(&self) -> bool {
        self.sunlight.pending()
    }

    /// The light (unit, toward it) the land's own shadow holds, or will once the worker's lands; none before the first.
    pub fn sun_light(&self) -> Option<Vec3> {
        self.sunlight.light()
    }

    /// The land's own shadow (`R8Unorm` on the field's grid), for tests and probes.
    pub fn sunlight_texture(&self) -> &wgpu::Texture {
        &self.sunlight.texture
    }

    /// The placed trees (`TREE_INSTANCE_BYTES` each, as many as [`TerrainLayer::tree_draw_arguments`] counts), for
    /// tests and probes to copy out; `None` without trees.
    pub fn tree_instances(&self) -> Option<&wgpu::Buffer> {
        self.trees.as_ref().map(|trees| &trees.trees)
    }

    /// The trees' indirect draw arguments (`DrawIndexedIndirectArgs`, five `u32`s: the second is the trees placed in
    /// the last frame), for tests and probes to copy out; `None` without trees (the Medium setting).
    pub fn tree_draw_arguments(&self) -> Option<&wgpu::Buffer> {
        self.trees.as_ref().map(|trees| &trees.args)
    }

    /// Hands in the detail photographs (encoded bytes of [`crate::textures::DETAIL_PHOTOS`], in order).
    pub fn set_detail_photos(
        &mut self,
        queue: &wgpu::Queue,
        photos: [&[u8]; 4],
    ) -> Result<(), String> {
        let packed = pack_detail(photos)?;
        let levels = mips_rgba8(&packed, DETAIL_SIZE, DETAIL_SIZE);
        let bytes: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
        write_levels(queue, &self.detail_texture, &bytes);
        Ok(())
    }

    /// Reflect another environment (the sky rebuilt its atlas).
    pub fn set_environment(&mut self, environment: &wgpu::BindGroup) {
        self.environment = environment.clone();
    }

    /// Shade the land from real-world surface maps (satellite colour, land cover) where they hold data, over the
    /// procedural cover (`SurfaceMaps`; native/docs/terrain/surface.md). Uploaded now; drawn from the next
    /// frame. A layer given none draws its procedural cover alone. The colour's mip chain is built on the CPU here (a
    /// few hundred milliseconds for 4096²); the maps replace any handed in before.
    pub fn set_surface_maps(
        &mut self,
        ctx: &mut RenderContext<'_>,
        maps: &crate::SurfaceMaps,
    ) -> Result<(), String> {
        self.set_surface_maps_marked(ctx, maps, &maps.cover)
    }

    /// As [`TerrainLayer::set_surface_maps`], the cover drawn from `classes` (`maps.size`² of them) in place of the
    /// maps' own: a past period's marks laid over them (`surface_maps::cover::PAST_COUNTRY`;
    /// native/docs/terrain/period.md, "The period's ground"), which the window's class means leave out.
    pub fn set_surface_maps_marked(
        &mut self,
        ctx: &mut RenderContext<'_>,
        maps: &crate::SurfaceMaps,
        classes: &[u8],
    ) -> Result<(), String> {
        if !maps.is_consistent() || classes.len() != maps.cover.len() {
            return Err("surface maps: buffers do not match their size".into());
        }
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        let largest = device.limits().max_texture_dimension_2d;
        if maps.size > largest {
            return Err(format!(
                "surface maps: {0}² texels, more than this device's {largest}² textures",
                maps.size
            ));
        }
        if self.surface.set_far(&device, &queue, maps, classes) {
            self.rebind_surface(&device);
        }
        let statistics = maps.statistics_with(classes);
        self.uniforms.surface = [
            maps.origin[0] as f32,
            maps.origin[1] as f32,
            maps.extent_m as f32,
            maps.size as f32,
        ];
        let [r, g, b] = statistics.haze;
        let stand = self.uniforms.surface_haze[3];
        self.uniforms.surface_haze = [r, g, b, stand];
        self.uniforms.cover_means = statistics.class_means;
        self.trees_stale = true;
        self.forest_shade_stale();
        self.surface_version += 1;
        Ok(())
    }

    /// Stop shading from surface maps: the procedural cover alone from the next frame (and the near tiles, if any).
    pub fn clear_surface_maps(&mut self, ctx: &mut RenderContext<'_>) {
        if !self.surface.has_far() {
            return;
        }
        if self.surface.clear_far() {
            self.rebind_surface(ctx.device);
        }
        self.uniforms.surface = [0.0; 4];
        self.uniforms.surface_haze = [0.0, 0.0, 0.0, self.uniforms.surface_haze[3]];
        self.uniforms.cover_means = Default::default();
        self.trees_stale = true;
        self.forest_shade_stale();
        self.surface_version += 1;
    }

    /// Whether real buildings stand on this land (a buildings layer drawn with it, `crate::buildings`), from the next
    /// frame: the surface maps' built-up class then draws the photograph with the ground's mottle and no lots, roofs or
    /// streets of its own, which would stand beside the real buildings and roads rather than under them
    /// (native/docs/terrain/surface.md). Off by default; kept across the maps' changes.
    pub fn set_buildings_stand(&mut self, stand: bool) {
        self.uniforms.surface_haze[3] = if stand { 1.0 } else { 0.0 };
    }

    /// Whether real buildings stand on this land (`set_buildings_stand`).
    pub fn buildings_stand(&self) -> bool {
        self.uniforms.surface_haze[3] > 0.5
    }

    /// Draw the land a past period leaves as country (the cover's `PAST_COUNTRY` mark) as `look`'s farmland, from the
    /// next frame; `None` draws a marked texel as today's class (`Period::Today`, and a battle map). Kept across the
    /// maps' changes (native/docs/terrain/period.md, "The period's ground").
    pub fn set_country(&mut self, look: Option<crate::CountryLook>) {
        self.country = look;
        let test_marks = self.uniforms.country[2];
        let row = look.map_or([0.0, 0.0, test_marks, 0.0], |l| l.uniform(test_marks));
        if row != self.uniforms.country {
            self.uniforms.country = row;
            self.trees_stale = true;
            self.forest_shade_stale();
        }
    }

    /// Whether a past period's country is drawn (`set_country`).
    pub fn draws_country(&self) -> bool {
        self.uniforms.country[3] > 0.5
    }

    /// The period's country drawn on marked land (`set_country`), if any.
    pub fn country(&self) -> Option<crate::CountryLook> {
        self.country
    }

    /// A test hook, never a setting: with the country drawn, the land takes every built-up texel as marked
    /// where a smooth noise of 700 m cells (fixed to the chart) lies under `share` (0-1; 0 none), as a host's marks
    /// would make it; for stills and measurements before the host marks the land itself.
    #[doc(hidden)]
    pub fn mark_country_for_tests(&mut self, share: f32) {
        self.uniforms.country[2] = share.clamp(0.0, 1.0);
        self.trees_stale = true;
        self.forest_shade_stale();
    }

    /// Whether surface maps are bound.
    pub fn has_surface_maps(&self) -> bool {
        self.surface.has_far()
    }

    /// The window's satellite photograph, for what is drawn over the land (`theatre::period_land` checks it): the
    /// colour array's view (layer 0 the window's maps: sRGB, premultiplied by coverage, mipped; layer 1 the near tiles),
    /// the maps' chart corner, side and texels, the haze to take out, and [`TerrainLayer::surface_version`]. None without
    /// maps.
    pub fn surface_photo(&self) -> Option<SurfacePhoto> {
        if !self.surface.has_far() {
            return None;
        }
        let [x, z, side, texels] = self.uniforms.surface;
        let [r, g, b, _] = self.uniforms.surface_haze;
        Some(SurfacePhoto {
            view: self.surface.views().0.clone(),
            origin: [f64::from(x), f64::from(z)],
            extent_m: f64::from(side),
            texels: texels as u32,
            haze: [r, g, b],
            version: self.surface_version,
        })
    }

    /// A number that changes whenever the surface's textures or the window's maps do.
    pub fn surface_version(&self) -> u64 {
        self.surface_version
    }

    /// Rewrites a rectangle of the cover from the next frame (`CoverPatch`: the window's maps', or a near tile's while
    /// its slot holds it): the host's marks of a past period's ground laid again (native/docs/terrain/period.md, "The
    /// period's
    /// ground"), without handing in the maps or the tile again. The trees are placed again and the forest's shade baked
    /// again about it. Needs no renderer context, so the frame loop can call it.
    pub fn queue_cover_patch(&mut self, patch: crate::CoverPatch) {
        self.surface.queue_patch(patch);
    }

    /// The host's number for the marks over near tile `index`'s cover (`NearTile::marks`, or the latest `CoverPatch`
    /// queued for it); none when the layer neither draws the tile nor waits for it.
    pub fn near_tile_marks(&self, index: [i32; 2]) -> Option<u64> {
        self.surface.near_marks(index)
    }

    /// Keeps the host's number for the marks it laid over the window's cover (opaque to the land; 0 until set).
    pub fn set_cover_marks(&mut self, marks: u64) {
        self.cover_marks = marks;
    }

    /// The host's number for the window cover's marks ([`TerrainLayer::set_cover_marks`]).
    pub fn cover_marks(&self) -> u64 {
        self.cover_marks
    }

    /// The chart rectangles (x0, z0, x1, z1) whose shading the cover rewrites `written` can change: each grown by two
    /// of its texels (the cover is read about a point, a texel or so), or by the country's reach where a past period's
    /// country is drawn (`Country::reach_m`).
    fn patched_chart(&self, written: &[(crate::CoverLayer, [u32; 4])]) -> Vec<[f64; 4]> {
        let [x, z, side, texels] = self.uniforms.surface;
        let near = self.surface.near_grid();
        written
            .iter()
            .filter_map(|(layer, [i, j, w, h])| {
                let (origin, texel) = match layer {
                    crate::CoverLayer::Window if texels > 0.0 => (
                        [f64::from(x), f64::from(z)],
                        f64::from(side) / f64::from(texels),
                    ),
                    crate::CoverLayer::Window => return None,
                    crate::CoverLayer::Near(index) => {
                        let grid = near?;
                        (
                            [
                                grid.origin[0] + f64::from(index[0]) * grid.tile_m,
                                grid.origin[1] + f64::from(index[1]) * grid.tile_m,
                            ],
                            grid.tile_m / f64::from(crate::surface_maps::NEAR_TILE_TEXELS),
                        )
                    }
                };
                // (The country's fields further: whole fields go with the marks about them.)
                let grow = (2.0 * texel).max(self.country.map_or(0.0, |c| c.country.reach_m()));
                Some([
                    origin[0] + f64::from(*i) * texel - grow,
                    origin[1] + f64::from(*j) * texel - grow,
                    origin[0] + f64::from(i + w) * texel + grow,
                    origin[1] + f64::from(j + h) * texel + grow,
                ])
            })
            .collect()
    }

    /// Shade the land near the camera from finer tiles of the same sources on `grid` (`NearGrid`, `NearTile`;
    /// native/docs/terrain/surface.md, "Near tiles"): they take the window maps' place inside every tile drawn,
    /// feathered where a neighbour is not. The two share textures: the grid's texels must match the window maps' side
    /// (`NearGrid::texels`), and the grid is set before the maps (a layer given maps alone holds no room for tiles).
    /// Tiles come with `queue_near_tile`.
    pub fn set_near_grid(
        &mut self,
        ctx: &mut RenderContext<'_>,
        grid: crate::NearGrid,
    ) -> Result<(), String> {
        if self.surface.set_near_grid(ctx.device, grid)? {
            self.rebind_surface(ctx.device);
        }
        self.trees_stale = true;
        self.forest_shade_stale();
        Ok(())
    }

    /// No near tiles from the next frame.
    pub fn clear_near_tiles(&mut self, ctx: &mut RenderContext<'_>) {
        if self.surface.clear_near() {
            self.rebind_surface(ctx.device);
        }
        self.trees_stale = true;
        self.forest_shade_stale();
    }

    /// The near tiles' grid, when set.
    pub fn near_grid(&self) -> Option<crate::NearGrid> {
        self.surface.near_grid()
    }

    /// Hands in near tile `index` (on the grid): uploaded by a following frame's `prepare`, a few a frame, and drawn
    /// from then, in the slot of whichever tile it replaces. Needs no renderer context, so the frame loop can call it.
    pub fn queue_near_tile(&mut self, index: [i32; 2], tile: std::sync::Arc<crate::NearTile>) {
        self.surface.queue_near(index, tile);
    }

    /// Whether near tile `index` is drawn or waiting for its upload.
    pub fn holds_near_tile(&self, index: [i32; 2]) -> bool {
        self.surface.near_holds(index)
    }

    /// Near tiles drawn, and waiting for their upload.
    pub fn near_tile_counts(&self) -> (usize, usize) {
        self.surface.near_counts()
    }

    /// The camera's chart point (x, z) at the last frame drawn, for a host placing near tiles about it.
    pub fn camera_chart(&self) -> Option<[f64; 2]> {
        self.camera_chart
    }

    /// Uploads every near tile handed in now rather than a few a frame (a layer being built, off the frame loop).
    pub fn flush_near_tiles(&mut self, ctx: &mut RenderContext<'_>) {
        let mut changed = Vec::new();
        while self.surface.upload_near(ctx.queue, &mut changed) > 0 {
            self.trees_stale = true;
        }
        self.forest_shade_changed(&changed);
    }

    /// Draw map features (roads, railways, rivers and canals) on the land from pages on `grid` (`FeatureGrid`,
    /// `FeaturePage`; native/docs/terrain/surface.md, "Map features on the ground"). Pages come with
    /// `queue_feature_page`.
    /// Refused on a device that allows fewer than 17 sampled textures a shader stage (the land's fragment stage
    /// samples 16 without them): the client's devices ask for 24 (`naval_effects::trial::limits`).
    pub fn set_feature_grid(
        &mut self,
        ctx: &mut RenderContext<'_>,
        grid: crate::FeatureGrid,
    ) -> Result<(), String> {
        let sampled = ctx.device.limits().max_sampled_textures_per_shader_stage;
        if !self.group_parts.features {
            return Err(format!(
                "map features: the device allows {sampled} sampled textures a stage, the land with them 17"
            ));
        }
        if self.features.set_grid(ctx.device, grid)? {
            self.rebind_surface(ctx.device);
        }
        if let Some(lamps) = &mut self.lamps {
            lamps.clear_pages();
        }
        self.trees_stale = true;
        self.forest_shade_stale();
        Ok(())
    }

    /// No map features from the next frame.
    pub fn clear_features(&mut self, ctx: &mut RenderContext<'_>) {
        if self.features.clear() {
            self.rebind_surface(ctx.device);
        }
        if let Some(lamps) = &mut self.lamps {
            lamps.clear_pages();
        }
        self.trees_stale = true;
        self.forest_shade_stale();
    }

    /// The map features' grid, when set.
    pub fn feature_grid(&self) -> Option<crate::FeatureGrid> {
        self.features.grid()
    }

    /// Hands in page `index` of `level` (on the grid): uploaded by a following frame's `prepare`, a few a frame, and
    /// drawn from then, in the slot of whichever page it replaces. Needs no renderer context.
    pub fn queue_feature_page(
        &mut self,
        level: usize,
        index: [i32; 2],
        page: std::sync::Arc<crate::FeaturePage>,
    ) {
        self.features.queue(level, index, page);
    }

    /// Whether page `index` of `level` is drawn or waiting for its upload.
    pub fn holds_feature_page(&self, level: usize, index: [i32; 2]) -> bool {
        self.features.holds(level, index)
    }

    /// Shows or hides the map features without dropping their pages (hidden, the land is drawn as with none: for
    /// comparisons and measurements).
    pub fn show_features(&mut self, shown: bool) {
        if self.features.shown != shown {
            self.features.shown = shown;
            self.trees_stale = true;
            self.forest_shade_stale();
        }
    }

    /// Map feature pages drawn, and waiting for their upload.
    pub fn feature_page_counts(&self) -> (usize, usize) {
        self.features.counts()
    }

    /// Uploads every page handed in now rather than a few a frame (a layer being built, off the frame loop).
    pub fn flush_feature_pages(&mut self, ctx: &mut RenderContext<'_>) {
        let mut changed = Vec::new();
        let mut uploaded = Vec::new();
        while self.features.upload(ctx.queue, &mut changed, &mut uploaded) > 0 {
            self.trees_stale = true;
        }
        self.forest_shade_changed(&changed);
        self.page_lamps(ctx.queue, &uploaded);
    }

    /// The lamps of the pages just uploaded, into their slots.
    fn page_lamps(
        &mut self,
        queue: &wgpu::Queue,
        uploaded: &[(usize, [i32; 2], Arc<crate::FeaturePage>)],
    ) {
        let (Some(lamps), Some(grid)) = (&mut self.lamps, self.features.grid()) else {
            return;
        };
        for (level, index, page) in uploaded {
            if *level == crate::night::LAMP_LEVEL {
                lamps.page(queue, &grid, *index, page);
            }
        }
    }

    /// How the land's lights burn at night (`NightLights`: the scenario's rule and the region's palette), from the
    /// next frame. Lit, sodium, until set.
    pub fn set_night_lights(&mut self, lights: crate::night::NightLights) {
        self.night_lights = lights;
        self.write_night();
    }

    pub fn night_lights(&self) -> crate::night::NightLights {
        self.night_lights
    }

    /// Shows or hides the lights at night without dropping their lamps (hidden, the land is drawn as with none: for
    /// comparisons and measurements).
    pub fn show_night_lights(&mut self, shown: bool) {
        if self.night_shown != shown {
            self.night_shown = shown;
            self.write_night();
        }
    }

    /// The theatre point (m) at the chart's (0, 0), where the street lights' lattice is fixed (`night::LATTICE_M`): the
    /// land's shading lays each lamp's pool on the road beneath it.
    pub fn set_night_origin(&mut self, origin: [f64; 2]) {
        let wrap = |v: f64| v.rem_euclid(crate::night::LATTICE_M) as f32;
        self.uniforms.night_grid = [wrap(origin[0]), wrap(origin[1]), 0.0, 0.0];
    }

    fn write_night(&mut self) {
        // The High setting's (its lamps made with the features' pages): the Medium land is drawn as before.
        let dark = if self.night_shown && self.lamps.is_some() {
            self.night
        } else {
            0.0
        };
        let (night, rule) = self.night_lights.uniforms(dark);
        self.uniforms.night = night;
        self.uniforms.night_rule = rule;
    }

    /// The red obstruction lights from the next frame: world x, height over the sea and world z of each (a buildings
    /// layer's `BuildingsLayer::beacons`). Nothing on the Medium setting.
    pub fn set_beacons(&mut self, world: &[[f64; 3]]) {
        let [ox, oz] = self.offset;
        let Some(lamps) = &mut self.lamps else {
            return;
        };
        let beacons = world
            .iter()
            .map(|p| {
                // A seed from where it stands, so it keeps its flashing's phase as the list changes about it.
                let seed =
                    ((p[0] * 7.31).round() as i64 ^ ((p[2] * 3.17).round() as i64) << 7) as u32;
                crate::night::Lamp::new(
                    crate::night::LampKind::Beacon,
                    0,
                    1.0,
                    seed.wrapping_mul(2_654_435_761) >> 8,
                    [(p[0] - ox) as f32, (p[2] - oz) as f32],
                    p[1] as f32,
                )
            })
            .collect();
        lamps.set_beacons(beacons);
    }

    /// The runways' lights from the next frame (`night::runway_lights` of each of an infrastructure layer's
    /// `InfraLayer::runways`): world x, height over the sea and world z of each, and its class. Nothing on the Medium
    /// setting.
    pub fn set_runway_lights(&mut self, world: &[([f64; 3], u8)]) {
        let [ox, oz] = self.offset;
        let Some(lamps) = &mut self.lamps else {
            return;
        };
        let lights = world
            .iter()
            .map(|(p, class)| {
                let seed =
                    ((p[0] * 5.13).round() as i64 ^ ((p[2] * 2.71).round() as i64) << 9) as u32;
                crate::night::Lamp::new(
                    crate::night::LampKind::Runway,
                    *class,
                    1.0,
                    seed.wrapping_mul(2_654_435_761) >> 8,
                    [(p[0] - ox) as f32, (p[2] - oz) as f32],
                    p[1] as f32,
                )
            })
            .collect();
        lamps.set_runways(lights);
    }

    /// The runways' lights held; none on the Medium setting.
    pub fn runway_lamps(&self) -> Option<usize> {
        self.lamps.as_ref().map(|l| l.runways())
    }

    /// The lamps held (on the pages, beacons) and drawn last frame; none on the Medium setting.
    pub fn lamp_counts(&self) -> Option<(usize, usize, usize)> {
        self.lamps.as_ref().map(|l| {
            let (pages, beacons) = l.held();
            (pages, beacons, l.drawn())
        })
    }

    /// Stand the trees where a canopy height map's crowns are, from pages on `grid` (`CanopyGrid`, `CanopyPage`;
    /// native/docs/terrain/trees.md, "Trees where real trees stand"). Pages come with `queue_canopy_page`. Refused on a
    /// land without trees (the Medium setting).
    pub fn set_canopy_grid(
        &mut self,
        ctx: &mut RenderContext<'_>,
        grid: crate::CanopyGrid,
    ) -> Result<(), String> {
        if self.trees.is_none() {
            return Err("the canopy: this land has no trees".into());
        }
        if self.canopy.set_grid(ctx.device, grid)? {
            self.rebind_canopy(ctx.device);
        }
        self.trees_stale = true;
        Ok(())
    }

    /// No canopy pages from the next frame: the land cover's trees everywhere.
    pub fn clear_canopy(&mut self, ctx: &mut RenderContext<'_>) {
        if self.canopy.clear() {
            self.rebind_canopy(ctx.device);
        }
        self.trees_stale = true;
    }

    /// The canopy's grid, when set.
    pub fn canopy_grid(&self) -> Option<crate::CanopyGrid> {
        self.canopy.grid()
    }

    /// Hands in canopy page `index` (on the grid): uploaded by a following frame's `prepare`, a few a frame, and read
    /// from then, in the slot of whichever page it replaces. Needs no renderer context.
    pub fn queue_canopy_page(&mut self, index: [i32; 2], page: std::sync::Arc<crate::CanopyPage>) {
        self.canopy.queue(index, page);
    }

    /// Whether canopy page `index` is read or waiting for its upload.
    pub fn holds_canopy_page(&self, index: [i32; 2]) -> bool {
        self.canopy.holds(index)
    }

    /// Reads or ignores the canopy without dropping its pages (ignored, the trees stand by the land cover alone: for
    /// comparisons and measurements).
    pub fn show_canopy(&mut self, shown: bool) {
        if self.canopy.shown != shown {
            self.canopy.shown = shown;
            self.trees_stale = true;
        }
    }

    /// Canopy pages read, and waiting for their upload.
    pub fn canopy_page_counts(&self) -> (usize, usize) {
        self.canopy.counts()
    }

    /// Uploads every canopy page handed in now rather than a few a frame (a layer being built, off the frame loop).
    pub fn flush_canopy_pages(&mut self, ctx: &mut RenderContext<'_>) {
        while self.canopy.upload(ctx.queue) > 0 {
            self.trees_stale = true;
        }
    }

    fn rebind_canopy(&mut self, device: &wgpu::Device) {
        let Some(trees) = &mut self.trees else {
            return;
        };
        trees.place_group = place_group(
            device,
            &trees.place_layout,
            [&trees.view, &self.instances, &trees.trees, &trees.args],
            self.canopy.view(),
        );
    }

    /// Draw the land near the camera with fine relief from pages on `grid` (`ReliefGrid`, `ReliefPage`;
    /// native/docs/terrain/ground.md, "Fine relief near the camera"): their residual heights added to the field's by
    /// the land's
    /// vertices (the level-0 patches within a quarter of level 0's range, and 3 km, drawn with twice the vertices a side
    /// over them), what stands on the land and its shadow, and their slopes to the field's by its fragments. Pages come with
    /// `queue_relief_page`. Refused where the layer has no room for them: the Medium setting, or a device allowing too
    /// few sampled textures a stage (the land with them takes 17, more with the map features and the ground materials;
    /// the client's devices ask for 24).
    pub fn set_relief_grid(
        &mut self,
        ctx: &mut RenderContext<'_>,
        grid: crate::ReliefGrid,
    ) -> Result<(), String> {
        if !self.group_parts.relief {
            return Err(format!(
                "fine relief: not on this layer (terrain setting {}, {} sampled textures a stage)",
                if self.settings.quality.relief {
                    "with it"
                } else {
                    "without it"
                },
                ctx.device.limits().max_sampled_textures_per_shader_stage
            ));
        }
        if self.relief.set_grid(ctx.device, grid)? {
            self.rebind_surface(ctx.device);
        }
        self.relief_changed();
        Ok(())
    }

    /// No fine relief from the next frame.
    pub fn clear_relief(&mut self, ctx: &mut RenderContext<'_>) {
        if self.relief.clear() {
            self.rebind_surface(ctx.device);
        }
        self.relief_changed();
    }

    /// Whether this layer can take fine relief (High, on a device with room for its texture).
    pub fn takes_relief(&self) -> bool {
        self.group_parts.relief
    }

    /// The fine relief's grid, when set.
    pub fn relief_grid(&self) -> Option<crate::ReliefGrid> {
        self.relief.grid()
    }

    /// Hands in relief page `index` (on the grid): uploaded by a following frame's `prepare`, a few a frame, and drawn
    /// from then, in the layer of whichever page it replaces. Needs no renderer context.
    pub fn queue_relief_page(&mut self, index: [i32; 2], page: Arc<crate::ReliefPage>) {
        self.relief.queue(index, page);
    }

    /// Whether relief page `index` is drawn or waiting for its upload.
    pub fn holds_relief_page(&self, index: [i32; 2]) -> bool {
        self.relief.holds(index)
    }

    /// The land's heights as drawn with the relief, on the CPU (none with it hidden or on no pages): the camera's
    /// clearance over a crest it lifts.
    pub fn relief_heights(&self) -> crate::ReliefHeights {
        self.relief.heights()
    }

    /// Relief pages drawn, waiting for their upload, and the level-0 patches drawn dense over them this view.
    pub fn relief_counts(&self) -> (usize, usize, usize) {
        let (drawn, waiting) = self.relief.counts();
        (drawn, waiting, self.dense.count)
    }

    /// Draws or hides the fine relief without dropping its pages (hidden, the land is drawn with none: for comparisons
    /// and measurements).
    pub fn show_relief(&mut self, shown: bool) {
        if self.relief.shown != shown {
            self.relief.shown = shown;
            self.relief_changed();
        }
    }

    /// Uploads every relief page handed in now rather than a few a frame (a layer being built, off the frame loop).
    pub fn flush_relief_pages(&mut self, ctx: &mut RenderContext<'_>) {
        let mut changed = Vec::new();
        while self.relief.upload(ctx.queue, &mut changed) > 0 {}
        if !changed.is_empty() {
            self.relief_changed();
        }
    }

    /// The land's heights changed near the camera: the patches are chosen again, the trees placed again and the
    /// forest's shade baked again.
    fn relief_changed(&mut self) {
        self.last_view = None;
        self.trees_stale = true;
        self.forest_shade_stale();
    }

    fn rebind_surface(&mut self, device: &wgpu::Device) {
        self.surface_version += 1;
        self.rebind(device);
    }

    /// The terrain group made again over what the layer holds now.
    fn rebind(&mut self, device: &wgpu::Device) {
        let (albedo, cover) = self.surface.views();
        self.terrain_group = self.group_parts.bind_group(
            device,
            albedo,
            cover,
            self.features.view(),
            &self.ground.view,
            self.relief.views(),
        );
    }

    /// Tile the ground materials under the land's colour close up (`GroundMaterials`; native/docs/terrain/ground.md,
    /// "Ground
    /// materials"), uploaded now and drawn from the next frame. Refused where the layer has no room for them: the Medium
    /// setting, or a device allowing too few sampled textures a stage (the land with them takes 17, 18 with the map
    /// features; the client's devices ask for 24). BC7 needs `TEXTURE_COMPRESSION_BC` on the device.
    pub fn set_ground_materials(
        &mut self,
        ctx: &mut RenderContext<'_>,
        materials: &crate::GroundMaterials,
    ) -> Result<(), String> {
        if !self.group_parts.materials {
            return Err(format!(
                "ground materials: not on this layer (terrain setting {}, {} sampled textures a stage)",
                if self.settings.quality.materials {
                    "with them"
                } else {
                    "without them"
                },
                ctx.device.limits().max_sampled_textures_per_shader_stage
            ));
        }
        if !materials.is_consistent() || materials.layers < crate::GroundMaterials::LAYERS {
            return Err("ground materials: levels do not match their size".into());
        }
        if materials.texels == crate::ground_materials::MaterialTexels::Bc7
            && !ctx
                .device
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            return Err("ground materials: BC7 on a device without BC textures".into());
        }
        self.ground.view = ground_texture(ctx.device, ctx.queue, materials);
        self.ground.bound = true;
        self.rebind_surface(ctx.device);
        Ok(())
    }

    /// Holds `materials` (those handed to [`TerrainLayer::set_ground_materials`]) for as long as this land lives.
    pub fn hold_ground_materials(&mut self, materials: Arc<crate::GroundMaterials>) {
        self.ground_held = Some(materials);
    }

    /// Whether this layer can take ground materials (High, on a device with room for their texture).
    pub fn takes_ground_materials(&self) -> bool {
        self.group_parts.materials
    }

    /// Whether ground materials are handed in (and so drawn, unless hidden).
    pub fn has_ground_materials(&self) -> bool {
        self.ground.bound
    }

    /// How strongly the ground materials show: their colour's contrast and their relief, 1 as each material's own
    /// (`GROUND_MATERIALS`), [`MATERIAL_STRENGTH`] by default (for tuning and comparisons).
    pub fn set_ground_material_strength(&mut self, strength: f32) {
        self.ground.strength = strength.max(0.0);
    }

    /// Shows or hides the ground materials without dropping them (hidden, the land draws as with none: for
    /// comparisons and measurements).
    pub fn show_ground_materials(&mut self, shown: bool) {
        self.ground.shown = shown;
    }

    /// Shade the sun by the clouds (naval-clouds' `CloudShadow::bind_group`), or stop (`None`).
    pub fn set_cloud_shadow(&mut self, bind_group: Option<&wgpu::BindGroup>) {
        self.cloud_bound = bind_group.is_some();
        self.cloud_group = bind_group
            .cloned()
            .unwrap_or_else(|| self.cloud_placeholder.clone());
    }

    /// The layout the cloud-shadow bind group must match (naval-clouds' own is group-equivalent).
    pub fn cloud_shadow_layout(&self) -> &wgpu::BindGroupLayout {
        &self.cloud_layout
    }

    /// The scene fog (the map's, or the port's).
    pub fn set_fog(&mut self, fog: FogParameters) {
        self.fog = fog;
    }

    /// The celestial light the land takes this frame (`VisualEnvironment.syncLighting` for lit meshes).
    pub fn set_light(&mut self, light: &TerrainLight) {
        let celestial = light.light;
        let (sun_share, fill_share, sky_share) = mesh_light_shares(
            f64::from(celestial.intensity),
            celestial.night,
            f64::from(light.flash),
        );
        let radiance = celestial.color * celestial.intensity * sun_share as f32;
        let fill = light.ambient * fill_share as f32;
        let (sky, ground) = naval_conditions::MESH_FILL_COLORS;
        let sky = naval_conditions::srgb_hex(sky);
        let ground = naval_conditions::srgb_hex(ground);
        let u = &mut self.uniforms;
        let direction = celestial.direction.normalize_or(Vec3::Y);
        u.sun_direction = direction.extend(0.0).to_array();
        u.sun_radiance = [radiance.x, radiance.y, radiance.z, sky_share as f32];
        u.fill_sky = [sky[0] * fill, sky[1] * fill, sky[2] * fill, 0.0];
        u.fill_ground = [ground[0] * fill, ground[1] * fill, ground[2] * fill, 0.0];
        u.equator[3] = light.flash;
        self.night = crate::night::night_of(direction.y, celestial.night);
        self.write_night();
    }

    fn write_uniforms(&mut self, uploads: &mut naval_render::Uploads) {
        let p = &self.prepared;
        let style = self.style;
        let u = &mut self.uniforms;
        u.field = [
            p.origin[0],
            p.origin[1],
            p.grid.cell as f32,
            self.settings.quality.lod.spacing as f32,
        ];
        u.grid = [
            p.grid.columns as f32,
            p.grid.rows as f32,
            p.coarse.columns as f32,
            p.coarse.rows as f32,
        ];
        u.params = [
            p.coarse.cell as f32,
            self.lod.ranges.first().copied().unwrap_or(10000.0) as f32,
            p.step,
            if self.settings.quality.detailed {
                1.0
            } else {
                0.0
            },
        ];
        u.offset = [
            self.offset[0] as f32,
            self.offset[1] as f32,
            self.tree_range,
            self.settings.debug.index(),
        ];
        u.sun_direction[3] = style.style.index() as f32;
        let fog = &self.fog;
        u.fog_color = [fog.color[0], fog.color[1], fog.color[2], fog.power];
        u.fog = [fog.start, fog.end, fog.sky_blend_distance, 1.0];
        u.shore = [style.shore[0], style.shore[1], style.shore[2], u.shore[3]];
        u.low = [style.low[0], style.low[1], style.low[2], 0.0];
        u.high = [style.high[0], style.high[1], style.high[2], 0.0];
        u.climate = [
            style.climate.snowline_m,
            style.climate.thaw,
            style.climate.leaf,
            style.climate.dry,
        ];
        u.equator[0] = style.equator[0];
        u.equator[1] = style.equator[1];
        u.equator[2] = if self.cloud_bound { 1.0 } else { 0.0 };
        // Which maps carry the trees' shadows (`naval_terrain::forest`).
        let [near, wide, aboard] = self.tree_casts.casts;
        u.fill_sky[3] = if near { 1.0 } else { 0.0 };
        u.fill_ground[3] = if wide { 1.0 } else { 0.0 };
        u.low[3] = if aboard { 1.0 } else { 0.0 };
        u.shore[3] = if self.trees_cast_land { 1.0 } else { 0.0 };
        self.surface.near_uniforms(&mut u.near, &mut u.near_slots);
        self.relief
            .uniforms(&mut u.relief, &mut u.relief_slots, &mut u.relief_real);
        self.features
            .uniforms(&mut u.features, &mut u.feature_origin, &mut u.feature_slots);
        u.materials = [
            f32::from(u8::from(self.ground.bound && self.ground.shown)),
            self.ground.strength,
            MATERIAL_FADE_M[0],
            MATERIAL_FADE_M[1],
        ];
        u.material_table = crate::ground_materials::material_table();
        u.cliff_rocks = crate::ground_materials::cliff_rocks();
        u.needles = crate::ground_materials::needle_floor();
        u.cliff_corner[3] = f32::from(u8::from(self.cliff_key.is_some() && self.rock_faces));
        // The near trees' reach (none while they are off: the impostors draw every tree), and the wind in the trees.
        let near = self.near_trees && self.trees.is_some();
        let reach = &self.near_view.reach;
        u.tree_near = [
            self.near_eye[0],
            self.near_eye[1],
            self.near_eye[2],
            if near { reach[0] } else { 0.0 },
        ];
        u.tree_lod = [reach[1], reach[3], reach[2], reach[3]];
        u.wind = [self.wind[0], self.wind[1], self.wind[2], self.wind_clock];
        uploads.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    /// Chooses the patches for the camera and uploads them, unless they were chosen for this very view already.
    fn select(
        &mut self,
        uploads: &mut naval_render::Uploads,
        camera: &naval_render::CameraUniforms,
    ) {
        let mut key = [0f32; 36];
        key[..16].copy_from_slice(&camera.view_projection);
        key[16..19].copy_from_slice(&camera.position);
        key[19] = camera.viewport[1];
        key[20] = camera.curvature;
        if self.last_view == Some(key) {
            return;
        }
        self.last_view = Some(key);
        let [ox, oz] = self.offset;
        let planes: Vec<[f64; 4]> = frustum_planes(Mat4::from_cols_array(&camera.view_projection))
            .into_iter()
            // World planes in chart metres: n · (p + offset) + c = n · p + (c + n · offset).
            .map(|[nx, ny, nz, c]| [nx, ny, nz, c + nx * ox + nz * oz])
            .collect();
        for (to, from) in self.tree_planes.iter_mut().zip(&planes) {
            *to = from.map(|v| v as f32);
        }
        self.tree_view.planes = crate::tree_shadows::moved_out(self.tree_planes, self.tree_margins);
        let position = camera.position;
        let view = LodView {
            position: [
                f64::from(position[0]) - ox,
                f64::from(position[1]),
                f64::from(position[2]) - oz,
            ],
            planes,
            fov_y: 2.0 * f64::from(camera.tan_half_fov_y).atan(),
            height_px: f64::from(camera.viewport[1]).max(1.0),
            submerged: position[1] < 0.0,
            curvature: f64::from(camera.curvature),
        };
        self.lod.dense = self.relief.extent();
        self.lod.relief_margin = f64::from(self.relief.peak());
        let count = self.lod.select(&view, &mut self.patches);
        self.patch_count = count;
        // The patches drawn dense, copied for the dense mesh's draw (past its capacity, drawn plain).
        let mut dense = Vec::new();
        for patch in self.patches[..count * PATCH_FLOATS]
            .as_chunks_mut::<PATCH_FLOATS>()
            .0
        {
            if patch[3] == crate::lod::DENSE_LAND {
                if dense.len() < DENSE_CAPACITY * PATCH_FLOATS {
                    dense.extend_from_slice(&[
                        patch[0],
                        patch[1],
                        patch[2],
                        crate::lod::DENSE_COPY,
                    ]);
                } else {
                    patch[3] = 1.0;
                }
            }
        }
        self.dense.count = dense.len() / PATCH_FLOATS;
        if !dense.is_empty() {
            uploads.write_buffer(&self.dense.instances, 0, bytemuck::cast_slice(&dense));
        }
        // The trees stand on the level-0 patches with land (at the front) and the level-1 ones, moved in behind them.
        let level_zero = self.lod.near_land;
        let mut front = level_zero;
        for k in level_zero..count {
            let at = k * PATCH_FLOATS;
            if self.patches[at + 2] != 1.0 || self.patches[at + 3] == 0.0 {
                continue;
            }
            if k != front {
                for f in 0..PATCH_FLOATS {
                    self.patches.swap(front * PATCH_FLOATS + f, at + f);
                }
            }
            front += 1;
        }
        self.tree_patches = front;
        // The trees thin out to none before the end of level 1, and no further than their range. A narrower lens than
        // the game's 52 degrees sees them further, by the square root of its magnification: as far as it magnifies would
        // place the trees of a whole hillside through binoculars.
        let zoom = (camera.tan_half_fov_y / TREE_LENS_TAN).sqrt();
        let level_one = self.lod.ranges.get(1).or(self.lod.ranges.first());
        self.tree_range = (level_one.copied().unwrap_or(0.0) * 0.92)
            .min(self.lod.tree_reach)
            .min(f64::from(TREE_RANGE_M / zoom.max(1e-3))) as f32;
        self.tree_view.eye = [
            view.position[0] as f32,
            view.position[1] as f32,
            view.position[2] as f32,
            zoom,
        ];
        let level_zero_range = self.lod.ranges.first().copied().unwrap_or(0.0) as f32;
        self.tree_view.lod = [TREE_FULL_M, self.tree_range, TREE_GROWTH, level_zero_range];
        // The near trees' meshes: binned from this view.
        let [reach, per_m] = crate::near_trees::near_reach(camera.tan_half_fov_y, TREE_LENS_TAN);
        self.near_view = crate::near_trees::NearView {
            planes: self.tree_view.planes,
            eye: self.tree_view.eye,
            reach: [
                reach,
                per_m,
                crate::near_trees::FULL_SHARE,
                crate::near_trees::FADE_SHARE,
            ],
            room: [0; 4],
        };
        self.near_eye = camera.position;
        // The ground cover's grid about the camera, while the camera is near enough the land to see it.
        let top = self
            .prepared
            .bounds
            .levels
            .last()
            .map_or(0.0, |l| l.max[0])
            .max(0.0);
        let near_land = view.position[1] < f64::from(COVER_REACH_M + top);
        // The grass's grid about the camera likewise, at the footprint a pixel takes.
        self.grass_view = crate::grass::GrassView::about(
            view.position,
            self.prepared.origin,
            self.tree_planes,
            2.0 * camera.tan_half_fov_y / camera.viewport[1].max(1.0),
            top,
        );
        self.grass_stale = true;
        let cell = f64::from(COVER_CELL_M);
        let reach = f64::from(COVER_REACH_M);
        let corner =
            |eye: f64, origin: f32| ((eye - f64::from(origin) - reach) / cell).floor() as f32;
        let biome = crate::species::Biome::of(self.style.style, &self.style.climate);
        self.tree_view.place = self
            .style
            .climate
            .place
            .map_or([0.0; 4], |[lat, lon]| [lat, lon, 1.0, 0.0]);
        // The biomes about the eye, on the chart's own lattice of cells (a tree's biome never changes as the grid
        // moves), worked out again only when the grid moves a cell.
        match self.style.climate.place {
            Some(place) => {
                use crate::species::{REGION_CELL_M, REGION_CELLS};
                let half = (REGION_CELLS / 2) as i32;
                let first = [
                    (view.position[0] / f64::from(REGION_CELL_M)).floor() as i32 - half,
                    (view.position[2] / f64::from(REGION_CELL_M)).floor() as i32 - half,
                ];
                let key = (first, place.map(f32::to_bits));
                if self.region_key != Some(key) {
                    self.region_key = Some(key);
                    self.tree_view.regions =
                        crate::species::region_grid(self.style.style, place, first);
                }
                self.tree_view.region_grid = [
                    first[0] as f32 * REGION_CELL_M,
                    first[1] as f32 * REGION_CELL_M,
                    REGION_CELL_M,
                    1.0,
                ];
            }
            None => self.tree_view.region_grid = [0.0; 4],
        }
        // The forest floor reads the same biomes as the trees stand by (`surface.wgsl`'s `floor_at`).
        self.uniforms.biome_grid = self.tree_view.region_grid;
        self.uniforms.biomes = self.tree_view.regions;
        self.uniforms.place = self.tree_view.place;
        // The cliffs' rock about the eye, on the chart's lattice likewise (none on a battle map: its style's own).
        match self.style.climate.place {
            Some(place) => {
                use crate::ground_materials::{CLIFF_CELL_M, CLIFF_CELLS};
                let half = (CLIFF_CELLS / 2) as i32;
                let first = [
                    (view.position[0] / f64::from(CLIFF_CELL_M)).floor() as i32 - half,
                    (view.position[2] / f64::from(CLIFF_CELL_M)).floor() as i32 - half,
                ];
                let key = (first, place.map(f32::to_bits));
                if self.cliff_key != Some(key) {
                    self.cliff_key = Some(key);
                    self.uniforms.cliff_grid = crate::ground_materials::cliff_grid(place, first);
                }
                self.uniforms.cliff_corner = [
                    first[0] as f32 * CLIFF_CELL_M,
                    first[1] as f32 * CLIFF_CELL_M,
                    CLIFF_CELL_M,
                    1.0,
                ];
            }
            None => self.uniforms.cliff_corner = [0.0; 4],
        }
        self.tree_view.biome = [
            biome as u32 as f32,
            COVER_REACH_M,
            corner(view.position[0], self.prepared.origin[0]),
            corner(view.position[2], self.prepared.origin[1]),
        ];
        let cells = (2.0 * reach / cell).ceil() as f32 + 1.0;
        // The culling sphere: the tallest tree the land's species can grow (its height's jitter, grown with distance),
        // its top leaning as far as a tree's may (`impostor.wgsl`'s `LEAN_MOST`) or the tallest palm's
        // (`PALM_LEAN_MOST`), never smaller than the web's sphere for a style's trees.
        let tallest = crate::species::tallest_m(biome) * 1.25 * TREE_GROWTH;
        let palms = crate::species::tallest_palm_m() * 1.25 * TREE_GROWTH;
        let lean = (tallest * TREE_LEAN_MOST).max(palms * TREE_PALM_LEAN_MOST);
        let (middle, radius) = (
            (tallest * 0.5).max(20.0),
            (tallest * 0.5 + lean + 15.0).max(45.0),
        );
        self.tree_view.cover = [
            if near_land { cells } else { 0.0 },
            COVER_CELL_M,
            middle,
            radius,
        ];
        self.trees_stale = true;
        if count > 0 {
            uploads.write_buffer(
                &self.instances,
                0,
                bytemuck::cast_slice(&self.patches[..count * PATCH_FLOATS]),
            );
        }
    }

    /// The patches that can throw a shadow into each of this frame's shadow maps the land casts into
    /// (`land_shadows`), for the caster passes.
    fn select_casters(&mut self, uploads: &mut naval_render::Uploads, maps: &[ShadowMap]) {
        self.caster_ranges = Default::default();
        if !self.settings.cast_shadows || self.settings.debug != TerrainDebug::Off {
            return;
        }
        let mut out = Vec::new();
        for map in maps.iter().filter(|m| m.on && m.drawn) {
            if matches!(map.kind, ShadowMapKind::View(_)) && !self.settings.cast_view_maps {
                continue;
            }
            let first = out.len() / PATCH_FLOATS;
            let room = (CASTER_CAPACITY - first).min(crate::land_shadows::LAND_MAP_PATCHES);
            out.extend(crate::land_shadows::land_patches(
                map,
                &self.lod,
                &self.prepared.bounds,
                self.offset,
                room,
            ));
            let count = out.len() / PATCH_FLOATS - first;
            self.caster_ranges[map.kind.index()] = (first as u32, count as u32);
        }
        if !out.is_empty() {
            uploads.write_buffer(&self.caster_instances, 0, bytemuck::cast_slice(&out));
        }
    }

    /// Recomputes the land's own shadow when the light has moved (`HeightfieldLandscape.update`), on the worker when the
    /// host handed one (`sunlight`).
    fn follow_sun(&mut self, queue: &wgpu::Queue) {
        let light = Vec3::from_array([
            self.uniforms.sun_direction[0],
            self.uniforms.sun_direction[1],
            self.uniforms.sun_direction[2],
        ]);
        self.sunlight.follow(light, &self.prepared, queue);
    }

    /// Places the trees for this frame's view (`tree_place.wgsl`), unless nothing they depend on has changed since the
    /// last placement.
    fn place_trees(&mut self, ctx: &mut PrepareContext<'_>) {
        let Some(trees) = &self.trees else {
            return;
        };
        let groups = self.tree_patches + self.tree_shadow_patches;
        if !self.trees_stale || groups == 0 || self.settings.debug != TerrainDebug::Off {
            return;
        }
        self.trees_stale = false;
        self.canopy
            .uniforms(&mut self.tree_view.canopy, &mut self.tree_view.canopy_slots);
        ctx.uploads
            .write_buffer(&trees.view, 0, bytemuck::bytes_of(&self.tree_view));
        // The instance count back to 0; the card's index count stays.
        ctx.encoder.clear_buffer(&trees.args, 4, Some(4));
        let mut pass = ctx
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("terrain tree placement"),
                timestamp_writes: ctx.timer.compute_pass("terrain tree placement"),
            });
        pass.set_pipeline(ctx.pipelines.compute(trees.place));
        pass.set_bind_group(0, &trees.place_group, &[]);
        pass.set_bind_group(1, &self.terrain_group, &[]);
        // One workgroup of `TREE_SLOTS`² threads a patch: the drawn ones, then the shadow-only ones.
        debug_assert_eq!(TREE_SLOTS, 16);
        pass.dispatch_workgroups(groups as u32, 1, 1);
        // The ground cover about the camera, into the same instances.
        let cells = self.tree_view.cover[0] as u32;
        if cells > 0 {
            pass.set_pipeline(ctx.pipelines.compute(trees.cover));
            pass.dispatch_workgroups(cells.div_ceil(8), cells.div_ceil(8), 1);
        }
        drop(pass);
        // The trees near the camera into their meshes' lists, in a pass of their own after the placement's count.
        if self.near_trees {
            trees.near.bin(ctx, self.near_view);
        }
    }

    /// The maps the trees cast into this frame (the near, the wide and the aboard, while on), and the level-0 patches inside them
    /// that the view left out, written after the drawn patches: their trees stand only so their shadows fall into the
    /// view. Places the trees again when the maps moved.
    fn fit_tree_shadows(
        &mut self,
        uploads: &mut naval_render::Uploads,
        maps: &[ShadowMap],
        eye: Vec3,
    ) {
        use crate::tree_shadows::{TreeCasts, moved_out, shadow_patches, tree_shadow_margins};
        let allowed = self.tree_shadows
            && self.trees.is_some()
            && self.settings.debug == TerrainDebug::Off
            && self.tree_patches > 0;
        let casts = TreeCasts::of(maps, allowed, eye, self.tree_view.eye[3]);
        // The land maps take the trees placed for the view (and for the near and wide maps), the view's sides moved out
        // toward the sun so the trees just beyond them stand too and throw their shadows in.
        let land = maps
            .iter()
            .find(|m| m.kind.is_land() && m.on)
            .filter(|_| allowed);
        self.trees_cast_land = land.is_some();
        let margins = land.map_or([0.0; 4], |map| {
            let toward = map.view.inverse().z_axis.truncate().normalize_or_zero();
            tree_shadow_margins(&self.tree_planes, toward, 2.0 * self.tree_view.cover[2])
        });
        if margins != self.tree_margins {
            self.tree_margins = margins;
            self.tree_view.planes = moved_out(self.tree_planes, margins);
            self.trees_stale = true;
        }
        if casts == self.tree_casts && !self.trees_stale {
            return;
        }
        self.trees_stale = true;
        let mut extra = Vec::new();
        if casts.any() {
            let room = (PATCH_CAPACITY - self.patch_count).min(TREE_SHADOW_PATCHES);
            extra = shadow_patches(
                maps,
                &casts,
                &self.lod,
                &self.prepared.bounds,
                self.offset,
                &self.patches[..self.tree_patches * PATCH_FLOATS],
                room,
            );
            if !extra.is_empty() {
                uploads.write_buffer(
                    &self.instances,
                    (self.patch_count * PATCH_FLOATS * 4) as u64,
                    bytemuck::cast_slice(&extra),
                );
            }
        }
        self.tree_shadow_patches = extra.len() / PATCH_FLOATS;
        self.tree_view.maps = casts.matrices;
        let flag = |on: bool| if on { 1.0 } else { 0.0 };
        self.tree_view.casts = [
            flag(casts.casts[0]),
            flag(casts.casts[1]),
            flag(casts.casts[2]),
            0.0,
        ];
        self.tree_view.shadow = [self.patch_count as f32, self.tree_patches as f32, 0.0, 0.0];
        self.tree_casts = casts;
    }

    /// The forest's shade must be baked again over the whole grid (the surface maps, the features' grid or the near
    /// tiles' grid changed).
    fn forest_shade_stale(&mut self) {
        let size = self.open_sky.size();
        if let Some(trees) = &mut self.trees {
            trees.shade.mark_all([size.width, size.height]);
        }
    }

    /// The forest's shade must be baked again where its inputs changed: chart rectangles (x0, z0, x1, z1) about the map
    /// features' pages and the near tiles that came into their slots or left them, each grown by how far its shading
    /// reaches (`FeatureGrid::reach`, `NearGrid::reach`).
    fn forest_shade_changed(&mut self, changed: &[[f64; 4]]) {
        let Some(trees) = &mut self.trees else {
            return;
        };
        let size = self.open_sky.size();
        let p = &self.prepared;
        for chart in changed {
            trees.shade.mark(crate::forest_shade::texels_of(
                *chart,
                p.origin,
                p.coarse.cell,
                [size.width, size.height],
            ));
        }
    }

    /// Bakes the forest's shade (`tree_shade.wgsl`) where it is waiting, after the open sky its masks read
    /// (`forest_shade`).
    fn bake_forest_shade(&mut self, ctx: &mut PrepareContext<'_>) {
        if let Some(trees) = &mut self.trees {
            trees.shade.bake(ctx, &self.terrain_group, &self.open_sky);
        }
    }

    fn bind_patches(&self, pass: &mut wgpu::RenderPass<'_>, instances: &wgpu::Buffer) {
        pass.set_bind_group(1, &self.terrain_group, &[]);
        pass.set_vertex_buffer(0, self.grid_vertices.slice(..));
        pass.set_vertex_buffer(1, instances.slice(..));
        pass.set_index_buffer(self.grid_indices.slice(..), wgpu::IndexFormat::Uint16);
    }

    /// The trees in the scene pass: the near trees' meshes, then the impostors (groups 1-3 as the land's).
    fn draw_trees(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let Some(trees) = &self.trees else {
            return;
        };
        if self.tree_patches + self.tree_shadow_patches == 0
            || self.settings.debug != TerrainDebug::Off
        {
            return;
        }
        pass.set_bind_group(1, &self.terrain_group, &[]);
        pass.set_bind_group(2, &self.environment, &[]);
        pass.set_bind_group(3, &self.cloud_group, &[]);
        // Every tree's depth, then their light where it stands (`near_trees::tree_pipelines`).
        let levels = [
            self.shows(TreePart::NearFull),
            self.shows(TreePart::NearReduced),
        ];
        for light in [false, true] {
            if self.near_trees {
                trees.near.draw(pass, ctx.pipelines, levels, light);
            }
            if self.shows(TreePart::Impostors) {
                pass.set_pipeline(ctx.pipelines.render(trees.pipelines[usize::from(light)]));
                pass.set_vertex_buffer(0, trees.vertices.slice(..));
                pass.set_vertex_buffer(1, trees.trees.slice(..));
                pass.set_index_buffer(trees.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed_indirect(&trees.args, 0);
            }
        }
        if self.shows(TreePart::Grass) {
            trees.grass.draw(pass, ctx.pipelines);
        }
    }
}

impl SceneLayer for TerrainLayer {
    fn label(&self) -> &str {
        "terrain"
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        // The impostor atlas's BC7 copy, once it is ready, in place of the bake (`impostors`).
        if let Some(texture) = self
            .trees
            .as_mut()
            .and_then(|trees| trees.impostors.refresh())
        {
            self.group_parts.views[8] = crate::impostors::atlas_view(&texture);
            self.rebind(ctx.device);
        }
        // The fine relief's pages handed in since the last frame, a few a frame, before the patches are chosen (which
        // draw dense over them and keep in view what they lift); the trees stand on them, and the forest's shade about
        // them is baked again.
        let mut relief_changed = Vec::new();
        if self.relief.upload(ctx.queue, &mut relief_changed) > 0 {
            self.last_view = None;
            self.trees_stale = true;
            self.forest_shade_changed(&relief_changed);
        }
        // The frame's patches and uniforms through its staging belt (`naval_render::upload`).
        self.select(ctx.uploads, ctx.camera);
        self.follow_sun(ctx.queue);
        self.select_casters(ctx.uploads, ctx.shadow_maps);
        // The near tiles handed in since the last frame, a few a frame, before the uniforms that say where they are.
        // The map features' pages handed in since the last frame, likewise (the trees keep off the roads: placed again).
        // The forest's shade is baked again about each, where the trees standing may have changed.
        let mut changed = Vec::new();
        let mut uploaded = Vec::new();
        if self.features.upload(ctx.queue, &mut changed, &mut uploaded) > 0 {
            self.trees_stale = true;
        }
        if self.surface.upload_near(ctx.queue, &mut changed) > 0 {
            self.trees_stale = true;
        }
        // The cover's rewrites (a past period's ground marked again), after the near tiles they may be for.
        let patched = self.surface.upload_patches(ctx.queue);
        if !patched.is_empty() {
            self.trees_stale = true;
            changed.extend(self.patched_chart(&patched));
        }
        // The canopy's pages likewise (the trees stand on its crowns: placed again).
        if self.canopy.upload(ctx.queue) > 0 {
            self.trees_stale = true;
        }
        self.forest_shade_changed(&changed);
        self.page_lamps(ctx.queue, &uploaded);
        let night = self.uniforms.night[0] > 0.0 && self.settings.debug == TerrainDebug::Off;
        if let Some(lamps) = &mut self.lamps {
            lamps.prepare(ctx.queue, &self.tree_view.planes, night);
        }
        let eye = ctx.camera.position;
        self.camera_chart = Some([
            f64::from(eye[0]) - self.offset[0],
            f64::from(eye[2]) - self.offset[1],
        ]);
        self.fit_tree_shadows(
            ctx.uploads,
            ctx.shadow_maps,
            Vec3::from_array(ctx.camera.position),
        );
        self.wind_clock = ctx.time.seconds as f32;
        self.write_uniforms(ctx.uploads);
        if !self.baked {
            // Bake once, before the first frame that samples the result.
            self.baked = true;
            let coarse = self.prepared.coarse;
            let mut pass = ctx
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("terrain open sky"),
                    timestamp_writes: ctx.timer.compute_pass("terrain open sky"),
                });
            pass.set_pipeline(ctx.pipelines.compute(self.bake));
            pass.set_bind_group(0, &self.bake_group, &[]);
            pass.set_bind_group(1, &self.bake_inputs, &[]);
            pass.dispatch_workgroups(coarse.columns.div_ceil(8), coarse.rows.div_ceil(8), 1);
        }
        self.bake_forest_shade(ctx);
        if let Some(trees) = &mut self.trees {
            trees.impostors.bake(ctx);
        }
        // The grass, where the view moved or the trees are placed again (what they stand on changed).
        if let Some(trees) = &mut self.trees
            && (self.grass_stale || self.trees_stale)
            && self.settings.debug == TerrainDebug::Off
        {
            self.grass_stale = false;
            trees
                .grass
                .place(ctx, &self.terrain_group, &self.grass_view);
        }
        self.place_trees(ctx);
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        // The lights on the land at night (`night`), over everything opaque.
        if phase == Phase::Transparent
            && let Some(lamps) = &self.lamps
        {
            lamps.draw(pass, ctx, &self.terrain_group);
            return;
        }
        if phase != Phase::Opaque || self.patch_count == 0 {
            return;
        }
        // The trees before the ground they stand on: their leaves are alpha-tested, and a GPU that sorts a tile's opaque
        // pixels before shading them (Apple's) shades every pixel of ground already drawn under an alpha-tested one;
        // drawn first, the trees' depth turns the ground's hidden pixels away instead.
        self.draw_trees(pass, ctx);
        pass.set_pipeline(ctx.pipelines.render(self.land));
        pass.set_bind_group(2, &self.environment, &[]);
        pass.set_bind_group(3, &self.cloud_group, &[]);
        // The patches drawn dense first: they are the nearest land, so their depth turns away the land behind them.
        if self.dense.count > 0 {
            self.bind_patches(pass, &self.dense.instances);
            pass.set_vertex_buffer(0, self.dense.vertices.slice(..));
            pass.set_index_buffer(self.dense.indices.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(
                0..4 * PATCH_QUADS * PATCH_QUADS * 6,
                0,
                0..self.dense.count as u32,
            );
        }
        self.bind_patches(pass, &self.instances);
        pass.draw_indexed(
            0..PATCH_QUADS * PATCH_QUADS * 6,
            0,
            0..self.patch_count as u32,
        );
    }

    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        // The land's patches for this map (`land_shadows`).
        let (first, count) = self.caster_ranges[map.kind.index()];
        if count > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.caster));
            self.bind_patches(pass, &self.caster_instances);
            pass.draw_indexed(0..PATCH_QUADS * PATCH_QUADS * 6, 0, first..first + count);
        }
        // The placed trees, into the near, wide and aboard maps, and into the land maps: the scene's own instances and count.
        if let Some(trees) = &self.trees
            && (self.tree_casts.casts_into(map.kind)
                || (map.kind.is_land() && self.trees_cast_land))
        {
            pass.set_pipeline(ctx.pipelines.render(trees.caster));
            pass.set_bind_group(1, &self.terrain_group, &[]);
            pass.set_vertex_buffer(0, trees.vertices.slice(..));
            pass.set_vertex_buffer(1, trees.trees.slice(..));
            pass.set_index_buffer(trees.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed_indirect(&trees.args, 0);
        }
    }
}

/// The attribute grid's stride, for hosts that read the open sky back.
pub const OPEN_SKY_STRIDE: u32 = ATTRIBUTE_STRIDE;
