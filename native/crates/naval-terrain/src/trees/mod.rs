//! Trees on the land (the web game's `terrain/TerrainTrees.ts`, extended): the card's geometry and the placement's
//! level of detail.
//!
//! The web drew a card for every slot of the nearest 256 level-0 patches with land, each placing itself in the vertex
//! stage. Here a compute pass (`shaders/tree_place.wgsl`) places the trees of every level-0 and level-1 patch with land
//! once a frame, thins them with distance and culls them to the view, chooses each one's species, and writes the
//! standing ones into an instance buffer that one indirect draw of a single card draws (native/docs/terrain/trees.md).
//! Since 2026-10-04 the card is an octahedral impostor of the species' 3D model ([`crate::species`],
//! [`crate::impostors`]) in place of the web's 2D atlas of painted trees.

pub mod canopy;
pub mod crown;
pub mod forest_shade;
pub mod grass;
pub mod impostors;
pub mod near_trees;
pub mod species;
pub mod tree_growth;
pub mod tree_meshes;
pub(crate) mod tree_shadows;

/// Tree slots along a patch's side: one per level-0 vertex spacing (`TREE_SLOTS`), one placement thread each.
pub const TREE_SLOTS: u32 = 16;
/// Indices of the one card every tree instances.
pub const TREE_CARD_INDICES: u32 = 6;
/// Most trees the instance buffer holds (48 bytes each); a frame's placement stops adding past it.
pub const TREE_CAPACITY: u32 = 1 << 18;
/// Bytes a placed tree takes in the instance buffer: three `vec4<f32>` (foot and height; the model's radius, species,
/// its turn and form packed in one float, and the open sky; tint and width).
pub const TREE_INSTANCE_BYTES: u64 = 48;
/// The most a tree's top leans, over its height (`shaders/impostor.wgsl`'s `LEAN_MOST`, which a test holds to it).
pub const TREE_LEAN_MOST: f32 = 0.16;
/// The most a palm's top leans, over its height (`PALM_LEAN_MOST`): a coconut palm's to 30 degrees and more.
pub const TREE_PALM_LEAN_MOST: f32 = 0.64;
/// The most a tree's crown is wider along its own x than its model (`STRETCH_MOST`).
pub const TREE_STRETCH_MOST: f32 = 0.3;

/// The trees' level of detail, at the game's 52° lens (a narrower lens sees further, by the square root of its
/// magnification).
///
/// Every slot keeps its tree out to [`TREE_FULL_M`]; beyond it a share `(TREE_FULL_M / d)²` stands (the trees on
/// screen keep their spacing in pixels), the stratified rank of each slot choosing which, and those still standing grow
/// by `d / TREE_FULL_M` up to [`TREE_GROWTH`] so the thinned forest keeps most of its cover; the painted canopy on the
/// ground carries the rest. Past two kilometres a grazing view thins them further, to a third (the rows stand one behind
/// another on screen), and the share comes down to a quarter by the end of level 0 (a level-1 patch holds only the
/// slots ranking below a quarter). They thin out to none at the end of their range: [`TREE_RANGE_M`] or 92% of level
/// 1's, whichever is nearer.
pub const TREE_FULL_M: f32 = 4500.0;
pub const TREE_GROWTH: f32 = 1.35;
pub const TREE_RANGE_M: f32 = 12000.0;
/// tan(26°): half the game's 52° lens, the lens the distances above are for.
pub const TREE_LENS_TAN: f32 = 0.487_732_6;

/// The one card (x across −0.5–0.5, y up 0–1) every tree instances, and its two triangles.
pub fn tree_card() -> ([f32; 8], [u32; 6]) {
    (
        [-0.5, 0.0, 0.5, 0.0, -0.5, 1.0, 0.5, 1.0],
        [0, 1, 3, 0, 3, 2],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_is_two_triangles() {
        let (vertices, indices) = tree_card();
        assert_eq!(vertices.len(), 8);
        assert_eq!(indices.len() as u32, TREE_CARD_INDICES);
    }
}
