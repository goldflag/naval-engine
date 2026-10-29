//! Lights on the land at night (native/docs/terrain/night.md): street lights along the paved roads, floodlit
//! quays and yards, red obstruction lights on what stands tall, an airfield's runway lights, drawn as small additive points the bloom picks up
//! (`lamps.wgsl`); the lit roads under them and the built-up land's glow beyond the map's pages are the land's own
//! shading (`night.wgsl`). Night only (nothing is drawn while the sun is up), the High setting's (the map features'
//! pages carry the lamps), presentation only: the simulation reads none of it.
//!
//! - **Lamps** ([`Lamp`], 16 bytes): the host makes them with each level-1 feature page
//!   ([`crate::FeaturePage::lamps`], page-local metres) and they are uploaded with it into its slot's share of one
//!   instance buffer, [`PAGE_LAMPS`] a slot, drawn a slot at a time where its page is in view; the runways' lights
//!   and the beacons the host hands in ([`crate::TerrainLayer::set_runway_lights`],
//!   [`crate::TerrainLayer::set_beacons`]) after them. The vertex stage stands each on the ground, keeps a
//!   street light only on built-up land, a flood off the water (but a pier's) and at full strength only near it, and
//!   sizes it at least [`MIN_PX`] pixels across, its light spread over them.
//! - **The ground under the street lights** is the land's own shading: it finds each road's lamps on their lattice
//!   ([`street_light`], [`LATTICE_M`]) from the pages alone.
//! - **The rule** ([`LightsRule`], a scenario's `lights`): lit; the 1942 dimout of the American west coast (street
//!   lights shielded, seen only from above, at a quarter of their light, no glow over the towns, most windows dark); a
//!   blackout (a rare leak only).
//! - **The palette** ([`LampPalette`]): sodium orange on the major roads and warmer whites on the rest (the Americas,
//!   Europe, the rest), or Japan's whites.

use bytemuck::{Pod, Zeroable};
use naval_render::{
    PipelineId, RenderContext, ScenePipeline, ShaderDefs, ShaderError, VertexLayout,
};

use crate::features::{FeatureGrid, FeaturePage, LEVELS, PAGE_TEXELS};

/// The feature level whose pages carry the lamps: 2 km pages, the block 16 km a side about the camera.
pub const LAMP_LEVEL: usize = LEVELS - 1;
/// Lamps a page holds at most (a dense city's 4 km² has about 2000).
pub const PAGE_LAMPS: usize = 4096;
/// Beacons drawn at most.
pub const BEACON_LAMPS: usize = 4096;
/// The least a lamp is drawn across, pixels: smaller, its light is spread over this.
pub const MIN_PX: f32 = 1.25;

/// What a lamp is (`lamps.wgsl`'s `LAMP_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LampKind {
    /// A street light over a paved road.
    Street = 0,
    /// A floodlight on a mast over a quay, a terminal or a yard.
    Flood = 1,
    /// The pool of light a flood lays on the ground beneath it (drawn flat).
    Pool = 2,
    /// A red obstruction light at the top of a chimney, a crane or a tall building (`height` over the sea).
    Beacon = 3,
    /// A runway's light ([`runway_lights`]): class 0 an edge light (white), 1 a threshold's (green); `height` over the
    /// sea.
    Runway = 4,
}

/// One light: where it stands, how high, and what it is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Lamp {
    /// x, z: page-local metres as the host makes it, chart metres once uploaded.
    pub at: [f32; 2],
    /// Its head's height over the ground (m); a beacon's over the sea.
    pub height: f32,
    /// The kind (bits 0-3), its class (4-7: a street light's road, [`road_class`]), its brightness in hundredths
    /// (8-15) and a seed (16-31).
    pub info: u32,
}

impl Lamp {
    pub fn new(
        kind: LampKind,
        class: u8,
        brightness: f32,
        seed: u32,
        at: [f32; 2],
        height: f32,
    ) -> Self {
        let bright = (brightness * 100.0).round().clamp(0.0, 255.0) as u32;
        Self {
            at,
            height,
            info: kind as u32
                | (u32::from(class.min(15)) << 4)
                | (bright << 8)
                | ((seed & 0xffff) << 16),
        }
    }

    pub fn kind(&self) -> u32 {
        self.info & 15
    }

    pub fn class(&self) -> u8 {
        ((self.info >> 4) & 15) as u8
    }

    pub fn brightness(&self) -> f32 {
        ((self.info >> 8) & 255) as f32 / 100.0
    }

    pub fn seed(&self) -> u32 {
        self.info >> 16
    }
}

/// The street lights' lattice repeats every this many metres (the least common multiple of their spacings,
/// [`street_light`]), so the land's shading finds a road's lamps from the chart point and the lattice's origin alone
/// ([`crate::TerrainLayer::set_night_origin`]).
pub const LATTICE_M: f64 = 138_600.0;

/// A paved road's street lights by its tone (`style_of`'s shade of its class, as the features' pages hold it) and half
/// width (m): their spacing (m), height (m) and brightness (1 a secondary road's), from a motorway's every 55 m 12 m up
/// to a service road's every 60 m; none on a road not lit (a driveway, an alley or a parking aisle, narrower; a paved
/// track). The land's shading reads the same from the pages (`night.wgsl`'s `street_light`), so the pools of light fall
/// under the lamps.
pub fn street_light(tone: f32, half: f32) -> Option<(f64, f32, f32)> {
    Some(if tone >= 0.95 {
        (55.0, 12.0, 1.4)
    } else if tone >= 0.85 {
        (45.0, 11.0, 1.3)
    } else if tone >= 0.70 {
        (40.0, 10.0, 1.2)
    } else if tone >= 0.60 {
        (42.0, 9.0, 1.0)
    } else if tone >= 0.47 {
        (45.0, 8.0, 0.9)
    } else if tone >= 0.42 {
        (50.0, 7.0, 0.7)
    } else if (0.30..0.38).contains(&tone) && half >= 2.2 {
        (60.0, 6.0, 0.5)
    } else {
        return None;
    })
}

/// A road's class for its street lights' colour: 0 a motorway, 1 a trunk road, 2 primary, 3 secondary, 4 tertiary,
/// 5 minor, 6 service, 7 anything else (8 the host's jetties, kept over the water). The major roads (0-3) take the
/// palette's major colour.
pub fn road_class(class: &str) -> u8 {
    match class {
        "motorway" => 0,
        "trunk" => 1,
        "primary" => 2,
        "secondary" => 3,
        "tertiary" => 4,
        "minor" => 5,
        "service" => 6,
        _ => 7,
    }
}

/// What a scenario's lights are (its `lights`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LightsRule {
    /// The land as lit as it is.
    #[default]
    Lit,
    /// The 1942 dimout of the American west coast: street lights shielded and dimmed, no glow over the towns, most
    /// windows dark.
    Dimout,
    /// A blackout: only a rare leak.
    Blackout,
}

/// The street lights' colours by where the land is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LampPalette {
    /// Sodium orange on the major roads, warm whites on the rest (the Americas, Europe, the rest).
    #[default]
    Sodium,
    /// Japan's: mercury white on the major roads, warm and cooler whites on the rest.
    White,
}

/// How the land's lights burn: the scenario's rule and the region's palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NightLights {
    pub rule: LightsRule,
    pub palette: LampPalette,
}

/// The rule as strengths, each 0-1 of the lit land's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightScales {
    /// Street lights and the lit roads under them.
    pub street: f32,
    /// The built-up land's glow beyond the pages.
    pub glow: f32,
    /// Floodlights and their pools.
    pub flood: f32,
    /// Obstruction lights.
    pub beacon: f32,
    /// The share of lamps that burn anyway, at a leak's strength, where the rule puts them out.
    pub leak: f32,
    /// The street lights and floods are shielded: seen only from above.
    pub shielded: bool,
    /// The share of a building's lit rooms left unshaded (`buildings.wgsl`'s `rule_light`).
    pub windows: f32,
}

impl LightsRule {
    pub fn scales(self) -> LightScales {
        match self {
            LightsRule::Lit => LightScales {
                street: 1.0,
                glow: 1.0,
                flood: 1.0,
                beacon: 1.0,
                leak: 0.0,
                shielded: false,
                windows: 1.0,
            },
            // The Western Defense Command's dimout from August 1942: street lights hooded and dimmed (a quarter of
            // their light, none upward or to seaward), signs and floodlights out or hooded, windows in sight of the sea
            // shaded; aviation's obstruction lights kept, dimmer.
            LightsRule::Dimout => LightScales {
                street: 0.25,
                glow: 0.06,
                flood: 0.2,
                beacon: 0.5,
                leak: 0.02,
                shielded: true,
                windows: 0.15,
            },
            LightsRule::Blackout => LightScales {
                street: 0.0,
                glow: 0.0,
                flood: 0.0,
                beacon: 0.0,
                leak: 0.004,
                shielded: false,
                windows: 0.03,
            },
        }
    }
}

impl NightLights {
    /// The land's uniforms: `night` (x the night, y the street lights, z the glow, w the floods) and `rule` (x the
    /// beacons, y the leaks' share, z 1 shielded, w the palette).
    pub(crate) fn uniforms(&self, night: f32) -> ([f32; 4], [f32; 4]) {
        let s = self.rule.scales();
        let palette = match self.palette {
            LampPalette::Sodium => 0.0,
            LampPalette::White => 1.0,
        };
        (
            [night, s.street, s.glow, s.flood],
            [
                s.beacon,
                s.leak,
                if s.shielded { 1.0 } else { 0.0 },
                palette,
            ],
        )
    }
}

/// How dark it is for the lights, 0 by day to 1: full under the moon, else as the sun sinks from 6° up to 2° below the
/// horizon (the buildings' windows light up alike).
pub fn night_of(direction_y: f32, moon: bool) -> f32 {
    if moon {
        1.0
    } else {
        ((0.10 - direction_y) / 0.14).clamp(0.0, 1.0)
    }
}

/// The lamps on the GPU and their pipeline.
pub(crate) struct NightLamps {
    pipeline: PipelineId,
    buffer: wgpu::Buffer,
    /// Lamps in each level-1 slot, then the beacons'.
    counts: Vec<u32>,
    /// Each slot's page in chart metres (x0, z0, x1, z1).
    boxes: Vec<[f32; 4]>,
    /// Each slot drawn this frame (in view, with lamps), the beacons and the runways' lights last.
    visible: Vec<bool>,
    beacons: Vec<Lamp>,
    runways: Vec<Lamp>,
    beacons_dirty: bool,
    /// The land's highest point (m): the slots' boxes reach it.
    top: f32,
}

const LAMP_BYTES: u64 = std::mem::size_of::<Lamp>() as u64;

impl NightLamps {
    pub fn new(
        ctx: &mut RenderContext<'_>,
        terrain_layout: &wgpu::BindGroupLayout,
        defs: ShaderDefs,
        top: f32,
    ) -> Result<Self, ShaderError> {
        let slots = crate::features::MAX_SLOTS as usize * crate::features::MAX_SLOTS as usize;
        let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
            format,
            offset,
            shader_location,
        };
        let pipeline = ctx.add_scene_pipeline(
            "terrain lamps",
            "naval-terrain/shaders/lamps.wgsl",
            ScenePipeline {
                vertex_buffers: vec![VertexLayout {
                    stride: LAMP_BYTES,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: vec![
                        attribute(wgpu::VertexFormat::Float32x2, 0, 0),
                        attribute(wgpu::VertexFormat::Float32, 8, 1),
                        attribute(wgpu::VertexFormat::Uint32, 12, 2),
                    ],
                }],
                bind_group_layouts: vec![terrain_layout.clone()],
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_write: false,
                // Added to what is behind, the target's alpha kept.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                defs,
                ..ScenePipeline::new("vs_lamp", "fs_lamp")
            },
        )?;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain lamps"),
            size: (slots * PAGE_LAMPS + BEACON_LAMPS) as u64 * LAMP_BYTES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            buffer,
            counts: vec![0; slots + 1],
            boxes: vec![[0.0; 4]; slots],
            visible: vec![false; slots + 1],
            beacons: Vec::new(),
            runways: Vec::new(),
            beacons_dirty: false,
            top,
        })
    }

    fn slots(&self) -> usize {
        self.boxes.len()
    }

    /// No page's lamps (the feature grid changed or went).
    pub fn clear_pages(&mut self) {
        let slots = self.slots();
        self.counts[..slots].fill(0);
    }

    /// Page `index` of the lamp level went into its slot: its lamps there, in chart metres.
    pub fn page(
        &mut self,
        queue: &wgpu::Queue,
        grid: &FeatureGrid,
        index: [i32; 2],
        page: &FeaturePage,
    ) {
        let [si, sj] = grid.slot(index);
        let slot = (sj * grid.slots + si) as usize;
        if slot >= self.slots() {
            return;
        }
        let side = grid.texel_m[LAMP_LEVEL] * f64::from(PAGE_TEXELS);
        let corner = [
            grid.origin[LAMP_LEVEL][0] + f64::from(index[0]) * side,
            grid.origin[LAMP_LEVEL][1] + f64::from(index[1]) * side,
        ];
        let n = page.lamps.len().min(PAGE_LAMPS);
        let lamps: Vec<Lamp> = page.lamps[..n]
            .iter()
            .map(|l| Lamp {
                at: [
                    (corner[0] + f64::from(l.at[0])) as f32,
                    (corner[1] + f64::from(l.at[1])) as f32,
                ],
                ..*l
            })
            .collect();
        if !lamps.is_empty() {
            queue.write_buffer(
                &self.buffer,
                (slot * PAGE_LAMPS) as u64 * LAMP_BYTES,
                bytemuck::cast_slice(&lamps),
            );
        }
        self.counts[slot] = n as u32;
        self.boxes[slot] = [
            corner[0] as f32,
            corner[1] as f32,
            (corner[0] + side) as f32,
            (corner[1] + side) as f32,
        ];
    }

    /// The beacons from the next frame (chart metres, heights over the sea).
    pub fn set_beacons(&mut self, beacons: Vec<Lamp>) {
        self.beacons = beacons;
        self.beacons_dirty = true;
    }

    /// The runways' lights from the next frame (chart metres, heights over the sea): first in the beacons' share of
    /// the buffer, the beacons after them to its end.
    pub fn set_runways(&mut self, runways: Vec<Lamp>) {
        self.runways = runways;
        self.runways.truncate(BEACON_LAMPS);
        self.beacons_dirty = true;
    }

    /// The runways' lights held.
    pub fn runways(&self) -> usize {
        self.runways.len()
    }

    /// Uploads changed beacons and picks the slots in view (`planes`: the frustum in chart metres, inside where
    /// n · p + w ≥ 0); none at all by day.
    pub fn prepare(&mut self, queue: &wgpu::Queue, planes: &[[f32; 4]; 6], night: bool) {
        let slots = self.slots();
        if self.beacons_dirty {
            self.beacons_dirty = false;
            let mut fixed = self.runways.clone();
            let room = BEACON_LAMPS - fixed.len();
            fixed.extend(self.beacons.iter().take(room));
            if !fixed.is_empty() {
                queue.write_buffer(
                    &self.buffer,
                    (slots * PAGE_LAMPS) as u64 * LAMP_BYTES,
                    bytemuck::cast_slice(&fixed),
                );
            }
            self.counts[slots] = fixed.len() as u32;
        }
        let (low, high) = (-20.0, self.top + 400.0);
        for slot in 0..slots {
            let [x0, z0, x1, z1] = self.boxes[slot];
            self.visible[slot] = night
                && self.counts[slot] > 0
                && planes.iter().all(|p| {
                    let x = if p[0] >= 0.0 { x1 } else { x0 };
                    let y = if p[1] >= 0.0 { high } else { low };
                    let z = if p[2] >= 0.0 { z1 } else { z0 };
                    // A margin for the Earth's curve and the lamps' quads.
                    p[0] * x + p[1] * y + p[2] * z + p[3] >= -60.0
                });
        }
        self.visible[slots] = night && self.counts[slots] > 0;
    }

    /// Lamps drawn last frame (in the slots drawn), for the host's status.
    pub fn drawn(&self) -> usize {
        self.counts
            .iter()
            .zip(&self.visible)
            .filter(|(_, v)| **v)
            .map(|(c, _)| *c as usize)
            .sum()
    }

    /// Lamps held, in pages and as beacons and runways' lights.
    pub fn held(&self) -> (usize, usize) {
        let slots = self.slots();
        (
            self.counts[..slots].iter().map(|c| *c as usize).sum(),
            self.counts[slots] as usize,
        )
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &naval_render::DrawContext<'_>,
        terrain: &wgpu::BindGroup,
    ) {
        if !self.visible.iter().any(|v| *v) {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(self.pipeline));
        pass.set_bind_group(1, terrain, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for (slot, (&count, &visible)) in self.counts.iter().zip(&self.visible).enumerate() {
            if visible && count > 0 {
                let first = (slot * PAGE_LAMPS) as u32;
                pass.draw(0..4, first..first + count);
            }
        }
    }
}

/// A runway's edge lights stand at most this far apart (m): the FAA's 200 ft.
pub const RUNWAY_EDGE_M: f64 = 60.0;
/// Its edge lights stand this far out from the paved edge (m), and its threshold lights this far apart across it.
const RUNWAY_EDGE_OUT_M: f64 = 3.0;
const RUNWAY_THRESHOLD_M: f64 = 3.0;
/// A runway light's height over the paint (m).
const RUNWAY_LIGHT_M: f64 = 0.4;

/// A runway's lights (an airfield's `infra::RunwaySpan`, from `InfraLayer::runways`): white edge lights along either
/// side, 3 m out from the paved edge, evenly from threshold to threshold at most [`RUNWAY_EDGE_M`] apart; and a bar of
/// green threshold lights across each end, 3 m apart, between them. Each in the span's own x and z, its height over
/// the sea, and its class (0 an edge light, 1 a threshold's). None for a span under 100 m.
pub fn runway_lights(span: &crate::infra::RunwaySpan) -> Vec<([f64; 3], u8)> {
    let [a, b] = span.ends;
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    let length = dx.hypot(dz);
    if length < 100.0 {
        return Vec::new();
    }
    let (ux, uz) = (dx / length, dz / length);
    let (nx, nz) = (-uz, ux);
    let out = f64::from(span.half) + RUNWAY_EDGE_OUT_M;
    let mut lights = Vec::new();
    let spans = (length / RUNWAY_EDGE_M).ceil() as usize;
    for i in 0..=spans {
        let t = i as f64 / spans as f64;
        let (x, z) = (a[0] + dx * t, a[2] + dz * t);
        let y = a[1] + (b[1] - a[1]) * t + RUNWAY_LIGHT_M;
        for side in [-1.0, 1.0] {
            lights.push(([x + nx * out * side, y, z + nz * out * side], 0));
        }
    }
    let across = ((out - 1.0) / RUNWAY_THRESHOLD_M).floor() as i32;
    for end in [a, b] {
        for j in -across..=across {
            let w = f64::from(j) * RUNWAY_THRESHOLD_M;
            lights.push((
                [end[0] + nx * w, end[1] + RUNWAY_LIGHT_M, end[2] + nz * w],
                1,
            ));
        }
    }
    lights
}

/// Where a chunk's obstruction lights stand (chunk-local x, z and the height over the sea, m): at the top of each
/// chimney and crane over 25 m, and at the roof's corners of anything standing 60 m or more (aviation's rule for a lit
/// obstruction is about 200 ft). Read from the mesh's vertices: their kind, and the highest and lowest of each building
/// (its hash) in each 25 m cell, so a tall roof's corners each take one and a crane's jib (no foot under it) none.
pub fn beacons_of(mesh: &crate::buildings::ChunkMesh) -> Vec<[f32; 3]> {
    use std::collections::HashMap;
    const CELL_M: f32 = 25.0;
    const KIND_CHIMNEY: u8 = crate::buildings::Kind::Chimney as u8;
    const KIND_CRANE: u8 = crate::buildings::Kind::Crane as u8;
    const KIND_LIGHTHOUSE: u8 = crate::buildings::Kind::Lighthouse as u8;
    // (low, high, x and z of the highest vertex)
    type Span = (f32, f32, f32, f32);
    let mut cells: HashMap<(u8, u8, i32, i32), Span> = HashMap::new();
    for v in &mesh.vertices {
        let kind = v.style[3] & 15;
        if kind == KIND_LIGHTHOUSE {
            continue;
        }
        let [x, y, z] = v.position;
        let key = (
            v.style[2],
            kind,
            (x / CELL_M).floor() as i32,
            (z / CELL_M).floor() as i32,
        );
        let e = cells.entry(key).or_insert((y, y, x, z));
        e.0 = e.0.min(y);
        if y > e.1 {
            *e = (e.0, y, x, z);
        }
    }
    let mut out: Vec<[f32; 3]> = cells
        .into_iter()
        .filter(|((_, kind, _, _), (low, high, _, _))| {
            let tall = high - low;
            match *kind {
                KIND_CHIMNEY | KIND_CRANE => tall >= 25.0,
                _ => tall >= 60.0,
            }
        })
        .map(|(_, (_, high, x, z))| [x, high + 0.5, z])
        .collect();
    // In a fixed order (the map's is not), and no two within a few metres.
    out.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[2].total_cmp(&b[2])));
    let mut kept: Vec<[f32; 3]> = Vec::with_capacity(out.len());
    for b in out {
        if !kept.iter().any(|k| (k[0] - b[0]).hypot(k[2] - b[2]) < 6.0) {
            kept.push(b);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lamp_packs_its_kind_class_brightness_and_seed() {
        let lamp = Lamp::new(LampKind::Flood, 3, 1.37, 0x1_2345, [1.0, 2.0], 25.0);
        assert_eq!(lamp.kind(), LampKind::Flood as u32);
        assert_eq!(lamp.class(), 3);
        assert!((lamp.brightness() - 1.37).abs() < 1e-6);
        assert_eq!(lamp.seed(), 0x2345);
        assert_eq!(std::mem::size_of::<Lamp>(), 16);
    }

    #[test]
    fn the_rules_dim_and_put_out() {
        let lit = LightsRule::Lit.scales();
        let dim = LightsRule::Dimout.scales();
        let black = LightsRule::Blackout.scales();
        assert_eq!(lit.street, 1.0);
        assert!(dim.street > 0.2 && dim.street < 0.31 && dim.shielded);
        assert!(dim.glow < 0.1 && dim.windows < 0.2);
        assert_eq!(black.street + black.glow + black.flood + black.beacon, 0.0);
        assert!(black.leak > 0.0 && black.leak < 0.01);
    }

    #[test]
    fn every_spacing_divides_the_lattice() {
        for tone in [1.0, 0.9, 0.75, 0.65, 0.55, 0.5, 0.45, 0.35] {
            let (spacing, _, _) = street_light(tone, 2.5).unwrap();
            assert_eq!(LATTICE_M % spacing, 0.0, "{spacing}");
        }
        assert_eq!(street_light(0.35, 1.8), None);
        assert_eq!(street_light(0.4, 3.0), None);
    }

    #[test]
    fn a_runway_is_lit_along_its_edges_and_across_its_ends() {
        let span = crate::infra::RunwaySpan {
            ends: [[100.0, 12.0, 200.0], [100.0, 14.0, 2200.0]],
            half: 22.5,
            designators: [(36, None), (18, None)],
        };
        let lights = runway_lights(&span);
        let edges: Vec<_> = lights.iter().filter(|l| l.1 == 0).collect();
        let bars: Vec<_> = lights.iter().filter(|l| l.1 == 1).collect();
        // 2 km at 60 m at most: 34 spans, 35 a side, 25.5 m off the centre line.
        assert_eq!(edges.len(), 70);
        assert!(
            edges
                .iter()
                .all(|l| ((l.0[0] - 100.0).abs() - 25.5).abs() < 1e-6)
        );
        let mut west: Vec<f64> = edges
            .iter()
            .filter(|l| l.0[0] < 100.0)
            .map(|l| l.0[2])
            .collect();
        west.sort_by(f64::total_cmp);
        assert!(west.windows(2).all(|w| w[1] - w[0] <= RUNWAY_EDGE_M + 1e-9));
        assert_eq!((west[0], west[34]), (200.0, 2200.0));
        // Across each end inside the edge lights, at the paint's height and a little over it.
        assert_eq!(bars.len(), 2 * 17);
        assert!(bars.iter().all(|l| (l.0[0] - 100.0).abs() < 25.0));
        assert!(bars.iter().all(|l| l.0[2] == 200.0 || l.0[2] == 2200.0));
        assert!(lights.iter().all(|l| l.0[1] > 12.0 && l.0[1] < 14.5));
        // A stub of a runway is not lit.
        let stub = crate::infra::RunwaySpan {
            ends: [[0.0, 0.0, 0.0], [0.0, 0.0, 60.0]],
            ..span
        };
        assert!(runway_lights(&stub).is_empty());
    }

    #[test]
    fn night_comes_as_the_sun_sinks() {
        assert_eq!(night_of(0.5, false), 0.0);
        assert_eq!(night_of(0.10, false), 0.0);
        assert_eq!(night_of(-0.04, false), 1.0);
        assert_eq!(night_of(0.5, true), 1.0);
    }
}
