//! Ports and infrastructure over a theatre's land (native/docs/terrain/infra.md): container
//! stacks on a port's terminals ([`containers`]), boats moored in a marina's slips ([`marina`]) and bridges over water
//! as structures ([`bridges`]), runways, taxiways and aprons laid on the ground ([`airfield`]), power lines on their
//! towers and communication masts ([`power`]), and in a past period the generated wartime bases standing then
//! ([`bases`]), each made per chunk of [`CHUNK_M`] from the map's data the host hands in, and drawn by [`InfraLayer`].
//! Presentation only: the simulation never knows of any of it.
//!
//! The chunks are the buildings' ([`crate::buildings::ChunkKey`]): [`Lod::Full`] a chunk within the near range, its
//! near detail made ([`Group::Detail`], [`Group::Near`]), [`Lod::Box`] one beyond it.

pub mod airfield;
pub mod bases;
pub mod bridges;
pub mod containers;
pub mod layer;
pub mod marina;
pub mod mesh;
pub mod period;
pub mod power;

pub use airfield::{Airfield, RunwaySpan};
pub use bridges::{BridgeLine, Under};
pub use layer::{InfraLayer, InfraSettings, InfraStatus};
pub use marina::Marina;
pub use mesh::{Group, InfraCounts, InfraMesh, InfraVertex, Material};
pub use power::Power;

use glam::Vec2;

use crate::buildings::tags::MapTags;
pub use crate::buildings::{CHUNK_M, ChunkKey, Lod};
use crate::buildings::{Period, hash01};

/// The share of a marina's boats moored in a past period: a few launches and small sailing boats.
pub const PERIOD_BOATS: f32 = 0.15;

/// What one chunk's infrastructure is made from (chunk-local metres, x east and z south from its corner).
pub struct ChunkInput<'a> {
    pub key: ChunkKey,
    /// The buildings' map tags of the chunk (`naval-client`'s `theatre::buildings::tags_for`).
    pub tags: &'a MapTags,
    /// The ground (m above the sea, the seabed under it).
    pub ground: &'a dyn Fn([f32; 2]) -> f32,
    pub marina: &'a Marina,
    /// Whether a point is open water (the boats').
    pub water: &'a dyn Fn(Vec2) -> bool,
    /// The bridges that touch the chunk, each whole, and the bridges' outlines about them.
    pub bridges: &'a [BridgeLine],
    pub bridge_outlines: &'a [Vec<[f32; 2]>],
    /// What lies under a point of a bridge: the map's water by its kind.
    pub under: &'a dyn Fn(Vec2) -> Under,
    /// The airfield about the chunk: its runways whole, its taxiways' and aprons' pieces.
    pub airfield: &'a Airfield,
    /// The power lines that touch the chunk, each whole, and the masts about it.
    pub power: &'a Power,
    /// The age it is drawn in: a past one has no container stacks (their yards stand empty) and few boats.
    pub period: Period,
    /// The generated bases about the chunk standing in the period (none today): each base's plan and its point in the
    /// chunk's metres.
    pub bases: &'a [bases::Placed<'a>],
}

/// One chunk's mesh: its airfield, stacks, boats, bridges, power lines and, in a past period, its generated bases.
pub fn build_chunk(input: &ChunkInput<'_>) -> InfraMesh {
    let mut m = mesh::MeshBuilder::new(input.key.lod == Lod::Full);
    if !input.airfield.is_empty() {
        let ground = |p: Vec2| (input.ground)(p.to_array());
        m.counts.runways += airfield::draw(&mut m, input.airfield, &ground);
    }
    let past = input.period.is_past();
    if past {
        let ground = |p: Vec2| (input.ground)(p.to_array());
        for base in input.bases {
            bases::draw::draw(&mut m, base, &ground);
        }
    }
    if !past {
        let containers = containers::stack(&mut m, input.tags, input.ground, input.key.index);
        m.counts.containers += containers;
    }
    let mut boats = marina::moor(input.marina, input.water, input.key.index);
    if past {
        let [i, j] = input.key.index.map(i64::from);
        boats.retain(|b| {
            let at = (b.centre * 10.0).as_i64vec2();
            hash01(i * 10_000 + at.x, j * 10_000 + at.y, 509) < PERIOD_BOATS
        });
    }
    marina::draw(&mut m, &boats);
    if !input.bridges.is_empty() {
        let ground = |p: Vec2| (input.ground)(p.to_array());
        bridges::build(
            &mut m,
            &bridges::BridgeInput {
                lines: input.bridges,
                outlines: input.bridge_outlines,
                ground: &ground,
                under: input.under,
            },
        );
    }
    if !input.power.is_empty() {
        let ground = |p: Vec2| (input.ground)(p.to_array());
        m.counts.towers += power::draw(&mut m, input.power, &ground);
    }
    m.finish()
}
