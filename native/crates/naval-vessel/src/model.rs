//! A ship model in CPU memory, ready to upload ([`VesselModel`]): the file's triangles in ship space, grouped by
//! material, its materials and its textures, and the [`Vessel`] its setup resolves to.

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{Context, Result, bail};
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Vec3};

use crate::images::{self, TextureImage};
use crate::read::{self, Alpha, Material, Resolver, Slot};
use crate::setup::Setup;
use crate::vessel::{Bounds, Vessel};

/// One vertex as the shaders read it (`VertexIn` in `shaders/vessel.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct Vertex {
    /// Ship space (m).
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    /// COLOR_0, sRGB-free linear RGBA in 0–255 (white without one).
    pub color: [u8; 4],
}

/// The triangles of one material.
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub material: usize,
    pub indices: Range<u32>,
}

/// A ship model loaded and resolved.
#[derive(Clone, Debug)]
pub struct VesselModel {
    pub vessel: Vessel,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<Draw>,
    pub materials: Vec<Material>,
    pub textures: Vec<TextureImage>,
    /// What was left out or guessed, worth telling whoever set the model up.
    pub notes: Vec<String>,
}

/// A resolver for a GLB that holds everything itself: any outside file is an error.
pub fn no_files(uri: &str) -> Result<Vec<u8>> {
    bail!("it names an outside file ({uri}): load it from its folder")
}

impl VesselModel {
    /// Loads the model `bytes` (a GLB, or a `.gltf` whose other files `resolver` reads) as `setup` places it. `name`
    /// names her in the [`Vessel`]. A file this crate cannot draw is refused with the reason.
    pub fn load(setup: &Setup, name: &str, bytes: &[u8], resolver: Resolver<'_>) -> Result<Self> {
        read::guarded(|| Self::build(setup, name, bytes, resolver))
    }

    fn build(setup: &Setup, name: &str, bytes: &[u8], resolver: Resolver<'_>) -> Result<Self> {
        setup.check()?;
        let file = read::parse(bytes, &mut *resolver)?;
        let mesh = read::scene_mesh(&file)?;
        let mut materials = read::materials(&file);
        read::check_uv_sets(&materials, &mesh)?;
        let (mut vessel, mut notes) =
            Vessel::resolve(setup, name, Bounds::of(mesh.positions.iter().copied()))?;
        notes.extend(mesh.notes.iter().cloned());
        let to_ship = vessel.model_to_ship;
        let turn = Mat3::from_mat4(to_ship).mul_scalar(1.0 / vessel.scale);
        let positions: Vec<Vec3> = mesh
            .positions
            .iter()
            .map(|&p| to_ship.transform_point3(p))
            .collect();
        let vertices: Vec<Vertex> = (0..positions.len())
            .map(|i| Vertex {
                position: positions[i].to_array(),
                normal: (turn * mesh.normals[i]).normalize_or(Vec3::Y).to_array(),
                uv0: mesh.uv0[i],
                uv1: mesh.uv1[i],
                color: mesh.colors[i],
            })
            .collect();

        // The file's materials, and glTF's default after them if a primitive has none.
        let default = materials.len();
        let mut indices = Vec::with_capacity(mesh.triangles() * 3);
        let mut draws = Vec::with_capacity(mesh.groups.len());
        for (material, triangles) in &mesh.groups {
            let start = indices.len() as u32;
            indices.extend_from_slice(triangles);
            draws.push(Draw {
                material: material.unwrap_or(default),
                indices: start..indices.len() as u32,
            });
        }
        if draws.iter().any(|d| d.material == default) {
            materials.push(Material::default());
        }
        vessel.cut_waterplane(&positions, &indices);

        // Each image once per colour space it is read in and whether its alpha is coverage (a base colour cut out or
        // blended), only if a drawn material maps it.
        let drawn: std::collections::HashSet<usize> = draws.iter().map(|d| d.material).collect();
        let mut textures = Vec::new();
        let mut decoded: HashMap<(usize, bool, bool), usize> = HashMap::new();
        for (m, material) in materials.iter_mut().enumerate() {
            for slot in Slot::ALL {
                let Some(map) = material.maps[slot as usize].as_mut() else {
                    continue;
                };
                if !drawn.contains(&m) {
                    material.maps[slot as usize] = None;
                    continue;
                }
                let coverage = slot == Slot::BaseColor && material.alpha != Alpha::Opaque;
                let key = (map.texture, slot.srgb(), coverage);
                map.texture = match decoded.get(&key) {
                    Some(&t) => t,
                    None => {
                        let (bytes, image_name) = read::image_bytes(&file, key.0, &mut *resolver)
                            .with_context(|| {
                            format!("material {:?}'s {} map", material.name, slot.name())
                        })?;
                        let texture =
                            images::decode(&bytes, &image_name, key.1, key.2, setup.texture_size)
                                .with_context(|| {
                                format!("material {:?}'s {} map", material.name, slot.name())
                            })?;
                        textures.push(texture);
                        decoded.insert(key, textures.len() - 1);
                        textures.len() - 1
                    }
                };
            }
        }
        Ok(Self {
            vessel,
            vertices,
            indices,
            draws,
            materials,
            textures,
            notes,
        })
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// The farthest any vertex stands from her origin (m).
    pub fn radius(&self) -> f32 {
        let b = self.vessel.bounds;
        b.min.abs().max(b.max.abs()).length()
    }
}
