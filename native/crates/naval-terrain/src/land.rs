//! The CPU half of a battle map's land, prepared away from the render thread.
//!
//! Building a map's land costs 200–700 ms on the CPU (`native/docs/terrain/layer.md`, "GPU cost"): the field's
//! preparation,
//! the mip chains, the noise tile, the trees' models, the detail photographs and the land's own shadow. [`PreparedLand`]
//! does all of it with no device; it is plain data (`Send`), so a host runs it on a worker thread while the rest of the
//! battle loads, and [`crate::TerrainLayer::from_land`] then only uploads and builds pipelines. The crate stays
//! portable: it spawns no thread itself.

use glam::Vec3;

use crate::field::PreparedField;
use crate::ground_materials::GroundMaterials;
use crate::heightfield::Heightfield;
use crate::noise::{NOISE_SIZE, terrain_noise};
use crate::textures::{DETAIL_SIZE, mips_rg16f, mips_rgba8, pack_detail};

/// Everything the land draws from that the CPU computes: the prepared field and every texture's mip chain.
pub struct PreparedLand {
    pub field: PreparedField,
    pub(crate) gradient_levels: Vec<Vec<u16>>,
    pub(crate) attribute_levels: Vec<Vec<u8>>,
    pub(crate) noise_levels: Vec<Vec<u8>>,
    /// The detail photographs packed and mipped; neutral (every channel at its mean) when none were handed in.
    pub(crate) detail_levels: Vec<Vec<u8>>,
    /// The land's own shadow for a light direction (unit, toward the light), when it was computed here.
    pub(crate) sunlight: Option<(Vec3, Vec<u8>)>,
    /// The ground materials, uploaded with the layer where it takes them (`with_ground_materials`).
    pub(crate) ground_materials: Option<std::sync::Arc<GroundMaterials>>,
}

impl PreparedLand {
    /// Prepares `field` (its heights, gradients, attributes and bounds) and every texture the land samples.
    pub fn new(field: &Heightfield) -> Self {
        Self::from_prepared(PreparedField::new(field))
    }

    /// As [`PreparedLand::new`] from an already prepared field.
    pub fn from_prepared(field: PreparedField) -> Self {
        let grid = field.grid;
        let gradient_levels = mips_rg16f(&field.gradients, grid.columns, grid.rows);
        let coarse = field.coarse;
        let attribute_levels = mips_rgba8(&field.attributes, coarse.columns, coarse.rows);
        let noise = terrain_noise();
        let noise_levels = mips_rgba8(&noise, NOISE_SIZE as u32, NOISE_SIZE as u32);
        // The trees' models, built once a process (the impostor atlas is baked from them on the GPU).
        let _ = crate::species::models();
        let neutral = vec![128u8; (DETAIL_SIZE * DETAIL_SIZE * 4) as usize];
        Self {
            field,
            gradient_levels,
            attribute_levels,
            noise_levels,
            detail_levels: mips_rgba8(&neutral, DETAIL_SIZE, DETAIL_SIZE),
            sunlight: None,
            ground_materials: None,
        }
    }

    /// Packs the detail photographs (encoded bytes of [`crate::textures::DETAIL_PHOTOS`], in order), as
    /// [`crate::TerrainLayer::set_detail_photos`] does on the render thread.
    pub fn with_detail_photos(mut self, photos: [&[u8]; 4]) -> Result<Self, String> {
        let packed = pack_detail(photos)?;
        self.detail_levels = mips_rgba8(&packed, DETAIL_SIZE, DETAIL_SIZE);
        Ok(self)
    }

    /// The ground materials the layer uploads with the land where it takes them (High, with room on the device;
    /// [`crate::TerrainLayer::set_ground_materials`] otherwise refuses them, and this leaves them out silently).
    pub fn with_ground_materials(mut self, materials: std::sync::Arc<GroundMaterials>) -> Self {
        self.ground_materials = Some(materials);
        self
    }

    /// Computes the land's own shadow for the scene's light (`CelestialLight::direction`, toward the light), which the
    /// layer otherwise computes on its first frame. The layer recomputes it only once the light moves past its
    /// tolerance.
    pub fn with_sun(mut self, direction: Vec3) -> Self {
        let direction = direction.normalize_or(Vec3::Y);
        let mut bytes = Vec::new();
        self.field.sunlight(
            [
                f64::from(direction.x),
                f64::from(direction.y),
                f64::from(direction.z),
            ],
            &mut bytes,
        );
        self.sunlight = Some((direction, bytes));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ridge() -> Heightfield {
        let (columns, rows) = (33u32, 25u32);
        let quanta = (0..rows)
            .flat_map(|j| {
                (0..columns).map(move |i| ((i as i32 - 16).abs() * -8 + 40 + j as i32) as i16)
            })
            .collect();
        Heightfield {
            columns,
            rows,
            cell: 20.0,
            origin_x: -320.0,
            origin_z: -240.0,
            step: 0.5,
            quanta,
        }
    }

    #[test]
    fn the_cpu_half_is_send_and_carries_every_level() {
        fn send<T: Send + 'static>(_: &T) {}
        let land = PreparedLand::new(&ridge());
        send(&land);
        let grid = land.field.grid;
        assert_eq!(
            land.gradient_levels.len() as u32,
            crate::textures::mip_count(grid.columns, grid.rows)
        );
        assert_eq!(land.gradient_levels[0], land.field.gradients);
        assert_eq!(land.attribute_levels[0], land.field.attributes);
        assert_eq!(
            land.detail_levels.len() as u32,
            crate::textures::mip_count(DETAIL_SIZE, DETAIL_SIZE)
        );
        assert!(land.sunlight.is_none());
    }

    #[test]
    fn the_sun_is_computed_as_the_layer_would() {
        let field = ridge();
        let light = Vec3::new(0.6, 0.3, -0.2);
        let land = PreparedLand::new(&field).with_sun(light);
        let (direction, bytes) = land.sunlight.as_ref().unwrap();
        assert!((direction.length() - 1.0).abs() < 1e-6);
        let mut expected = Vec::new();
        let d = light.normalize();
        land.field.sunlight(
            [f64::from(d.x), f64::from(d.y), f64::from(d.z)],
            &mut expected,
        );
        assert_eq!(bytes, &expected);
    }
}
