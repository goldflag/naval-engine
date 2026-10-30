//! The towns' light in the sky at night (native/docs/terrain/night.md, "The towns' light in the sky"): the light the
//! lit land sends up, which the air over a town scatters toward the eye (the glow over a town on the horizon, the
//! brighter sky inside one) and a cloud's base takes from below. One source for both, the land's own glow: the host
//! hands the sky the land's lit map ([`GroundGlow`], `naval_terrain::TerrainLayer::ground_glow`, its cells' share of
//! wholly built-up land) and each frame how it burns ([`GlowLight`]: the land's glow's radiance by the scenario's rule
//! and the night). A compute pass (`shaders/glow_field.wgsl`) lays the light in the air over a grid about the window,
//! into the sky's aerial buffer after the volume (`naval_sky::aerial::GLOW_OFFSET`), when the map or the air changes;
//! the aerial perspective (`shaders/aerial.wgsl`) and the clouds' march (`shaders/march.wgsl`) read it. By day, at
//! sea, on the Medium land and with the lights out the header says so and nothing is read.

use bytemuck::{Pod, Zeroable};
use naval_sky::Sky;
use naval_sky::aerial::{
    GLOW_FIELD_BYTES, GLOW_HEADER_BYTES, GLOW_LEVELS, GLOW_OFFSET, GLOW_SIDE, GlowHeader,
};

use crate::gpu;

/// A land's lit map: `size`² cells over the square of `extent_m` metres of chart whose corner is chart point `origin`
/// (cell (i, j) covers chart x from `origin[0] + i · extent_m / size`, z likewise, row j = 0 at the smallest z), each
/// its share (0-1) of the light wholly built-up land sends up.
#[derive(Clone, Debug, PartialEq)]
pub struct GroundGlow {
    pub origin: [f64; 2],
    pub extent_m: f64,
    pub size: u32,
    pub lit: Vec<f32>,
    /// Changes with the map (the land's surface maps): the light in the air is laid again.
    pub version: u64,
}

/// How the lit land burns this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowLight {
    /// World = chart + offset (the land's).
    pub offset: [f64; 2],
    /// The radiance of wholly lit land (linear rgb): the land's glow's colour and luminance by the scenario's rule, at
    /// the night's strength. Zero puts the light out.
    pub radiance: [f32; 3],
}

/// The light the lit land sends up over the light its glow shows (`GroundGlow`'s cells, at the land's glow's
/// radiance): the land draws the light its ground reflects, Garstang's share G(1 − F) of the lamps' light, and the lamps
/// send a share F up unreflected besides (his G 0.15 and F 0.1, as `glow_field.wgsl`'s emission).
pub const UPWARD: f32 = (0.15 * 0.9 + 0.1) / (0.15 * 0.9);

/// The most lit cells a map hands the sky (64² cells).
pub const MAX_SOURCES: usize = 4096;
/// The grid of the light in the air reaches this share of the map's side past each of its edges: a town near the
/// window's edge lights the air beyond it.
const REACH: f64 = 0.25;
/// A cell lit less than this sends nothing worth the sum.
const LEAST_LIT: f32 = 0.002;
/// The atmosphere's scale height (m) for the light's path through it: its molecules' 8 km and the clear-air aerosol's
/// 1.2 km, near the sea mostly the aerosol's.
const ATMOSPHERE_SCALE_M: f32 = 2000.0;
/// The air the light was laid through changes this much (a share) before it is laid again.
const AIR_TOLERANCE: f32 = 0.04;

/// `GlowParams` in `shaders/glow_field.wgsl`, 48 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GlowParams {
    frame: [f32; 4],
    air: [f32; 4],
    counts: [u32; 4],
}

/// The grid of the light in the air for `glow`'s map, in its chart: x, z of its corner, a column's side, columns.
pub fn field_frame(glow: &GroundGlow) -> [f64; 4] {
    let reach = glow.extent_m * REACH;
    [
        glow.origin[0] - reach,
        glow.origin[1] - reach,
        (glow.extent_m + 2.0 * reach) / f64::from(GLOW_SIDE),
        f64::from(GLOW_SIDE),
    ]
}

/// The map's lit cells as the pass reads them: chart x, z of each one's middle, its light sent up per unit radiance of
/// wholly lit land (π × [`UPWARD`] × its share × its area) and the mean square distance across it.
pub fn sources(glow: &GroundGlow) -> Vec<[f32; 4]> {
    let n = glow.size as usize;
    if n == 0 || glow.lit.len() != n * n || glow.extent_m <= 0.0 {
        return Vec::new();
    }
    let cell = glow.extent_m / n as f64;
    let area = (cell * cell) as f32;
    let mut out = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let lit = glow.lit[j * n + i];
            if lit > LEAST_LIT && out.len() < MAX_SOURCES {
                out.push([
                    (glow.origin[0] + (i as f64 + 0.5) * cell) as f32,
                    (glow.origin[1] + (j as f64 + 0.5) * cell) as f32,
                    std::f32::consts::PI * UPWARD * lit.min(1.0) * area,
                    area / 6.0,
                ]);
            }
        }
    }
    out
}

/// The light in the air's header for `glow` burning by `light` (or out).
pub fn header(glow: Option<(&GroundGlow, GlowLight)>) -> GlowHeader {
    let Some((map, light)) = glow else {
        return GlowHeader::default();
    };
    let burns = light.radiance.iter().any(|&c| c > 0.0) && map.lit.iter().any(|&l| l > LEAST_LIT);
    if !burns {
        return GlowHeader::default();
    }
    let frame = field_frame(map);
    GlowHeader {
        frame: [
            (frame[0] + light.offset[0]) as f32,
            (frame[1] + light.offset[1]) as f32,
            frame[2] as f32,
            frame[3] as f32,
        ],
        radiance: [light.radiance[0], light.radiance[1], light.radiance[2], 1.0],
    }
}

/// Whether the clouds take the towns' light otherwise under `after` than under `before` (each the map and how it burns,
/// or none): its header (where the field lies, the lit land's radiance, whether it burns) or, while it burns, the map
/// (its version: the field laid again). A held or paused picture's converged clouds then march again
/// (`CloudLayer::air_changed`): their bases and the air in front of them hold the old light.
pub(crate) fn relights_clouds(
    before: Option<(&GroundGlow, GlowLight)>,
    after: Option<(&GroundGlow, GlowLight)>,
) -> bool {
    let header_before = header(before);
    header_before != header(after)
        || (header_before.radiance[3] > 0.0
            && before.map(|(map, _)| map.version) != after.map(|(map, _)| map.version))
}

/// The air the light crosses, as the pass takes it: the haze's extinction at the sea and its scale height, the
/// atmosphere's (as a view sees it) and its scale height.
fn air_of(sky: &Sky) -> [f32; 4] {
    let haze = sky.weather_haze();
    let at_sea = haze.extinction_at_sea(sky.air());
    let atmosphere = (haze.sighting_extinction() - at_sea).max(0.0);
    [
        at_sea as f32,
        haze.scale_height.max(1.0) as f32,
        atmosphere as f32,
        ATMOSPHERE_SCALE_M,
    ]
}

fn air_moved(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter()
        .zip(b.iter())
        .any(|(x, y)| (x - y).abs() > AIR_TOLERANCE * x.abs().max(y.abs()).max(1e-7))
}

/// The pass that lays the light in the air, on its own pipeline (the aerial perspective's registry entry).
pub(crate) struct GlowPass {
    pub(crate) layout: wgpu::BindGroupLayout,
    pub(crate) pipeline: Option<wgpu::ComputePipeline>,
    params: wgpu::Buffer,
    sources: wgpu::Buffer,
    group: Option<wgpu::BindGroup>,
    /// The map and the air the field holds (none laid yet).
    laid: Option<(u64, [f32; 4])>,
    /// The header last written.
    written: Option<GlowHeader>,
}

impl GlowPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = gpu::layout(
            device,
            "ground glow",
            &[
                gpu::uniform_entry(0),
                gpu::storage_buffer_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        );
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ground glow"),
            size: std::mem::size_of::<GlowParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sources = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ground glow sources"),
            size: (MAX_SOURCES * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline: None,
            params,
            sources,
            group: None,
            laid: None,
            written: None,
        }
    }

    /// Lays the field again at the next encode (its pass was rebuilt).
    pub(crate) fn lay_again(&mut self) {
        self.laid = None;
    }

    /// Writes the header when it changed and lays the field again when the map or the air did. Before the passes that
    /// read it: the environment's bake (the clouds' bases the sea reflects), the aerial perspective, the clouds' march.
    /// The header's write lands before every pass of the frame (the uploads' ordering), so a field laid after a pass
    /// that reads it would meet the new header with the old field.
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &mut naval_render::UploadSink<'_>,
        encoder: &mut wgpu::CommandEncoder,
        sky: &Sky,
        glow: Option<(&GroundGlow, GlowLight)>,
        timer: Option<&naval_render::GpuTimer>,
    ) {
        let header = header(glow);
        if self.written != Some(header) {
            sky.aerial().write_glow_header(queue, &header);
            self.written = Some(header);
        }
        let (Some((map, _)), Some(pipeline)) = (glow, &self.pipeline) else {
            return;
        };
        if header.radiance[3] <= 0.0 {
            return;
        }
        let air = air_of(sky);
        let fresh = self
            .laid
            .is_some_and(|(version, laid)| version == map.version && !air_moved(laid, air));
        if fresh {
            return;
        }
        let sources = sources(map);
        let frame = field_frame(map);
        let points = GLOW_SIDE * GLOW_SIDE * GLOW_LEVELS;
        let params = GlowParams {
            frame: frame.map(|v| v as f32),
            air,
            counts: [sources.len() as u32, points, 0, 0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        if !sources.is_empty() {
            queue.write_buffer(&self.sources, 0, bytemuck::cast_slice(&sources));
        }
        let group = self.group.get_or_insert_with(|| {
            gpu::bind_group(
                device,
                "ground glow",
                &self.layout,
                &[
                    (0, self.params.as_entire_binding()),
                    (1, self.sources.as_entire_binding()),
                    (
                        2,
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &sky.aerial().buffer,
                            offset: GLOW_OFFSET + GLOW_HEADER_BYTES,
                            size: std::num::NonZeroU64::new(GLOW_FIELD_BYTES),
                        }),
                    ),
                ],
            )
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ground glow"),
            timestamp_writes: timer.and_then(|timer| timer.compute_pass("ground glow")),
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*group, &[]);
        pass.dispatch_workgroups(gpu::workgroups(points, 64), 1, 1);
        self.laid = Some((map.version, air));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(lit: f32) -> GroundGlow {
        GroundGlow {
            origin: [-32_000.0, -32_000.0],
            extent_m: 64_000.0,
            size: 64,
            lit: (0..64 * 64)
                .map(|k| if k % 64 < 8 { lit } else { 0.0 })
                .collect(),
            version: 1,
        }
    }

    #[test]
    fn params_match_the_wgsl() {
        assert_eq!(std::mem::size_of::<GlowParams>(), 48);
        let module = crate::shaders::compose(
            crate::shaders::GLOW_FIELD_WGSL,
            crate::shaders::GLOW_FIELD_PATH,
            &[],
        )
        .unwrap_or_else(|report| panic!("{report}"));
        let params = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("GlowParams"))
            .expect("GlowParams in the pass");
        assert_eq!(params.1.inner.size(module.to_ctx()), 48);
    }

    #[test]
    fn only_lit_cells_send_light() {
        let glow = map(0.5);
        let s = sources(&glow);
        assert_eq!(s.len(), 8 * 64);
        // A 1 km cell half lit: π × 0.5 × 1 km², and the lamps' own light sent up.
        assert!((UPWARD - 1.7407).abs() < 1e-3);
        assert!((s[0][2] - std::f32::consts::PI * UPWARD * 0.5 * 1.0e6).abs() < 4.0);
        assert_eq!([s[0][0], s[0][1]], [-31_500.0, -31_500.0]);
        assert!(sources(&map(0.0)).is_empty());
    }

    #[test]
    fn nothing_burns_by_day_or_with_nothing_lit() {
        let light = GlowLight {
            offset: [100.0, -50.0],
            radiance: [0.04, 0.02, 0.005],
        };
        let lit = header(Some((&map(1.0), light)));
        assert_eq!(lit.radiance[3], 1.0);
        // The grid reaches a quarter of the side past the map, in the world's metres.
        assert_eq!(
            lit.frame,
            [-48_000.0 + 100.0, -48_000.0 - 50.0, 96_000.0 / 80.0, 80.0]
        );
        let day = GlowLight {
            radiance: [0.0; 3],
            ..light
        };
        assert_eq!(header(Some((&map(1.0), day))), GlowHeader::default());
        assert_eq!(header(Some((&map(0.0), light))), GlowHeader::default());
        assert_eq!(header(None), GlowHeader::default());
    }

    #[test]
    fn a_change_of_the_light_marches_converged_clouds_again() {
        let light = GlowLight {
            offset: [100.0, -50.0],
            radiance: [0.04, 0.02, 0.005],
        };
        let lit = map(1.0);
        let on = Some((&lit, light));
        assert!(!relights_clouds(on, on));
        assert!(!relights_clouds(None, None));
        // Hidden or shown, a dimmer rule, another palette, the land moved under the world.
        assert!(relights_clouds(on, None) && relights_clouds(None, on));
        let dim = GlowLight {
            radiance: [0.0024, 0.0012, 0.0003],
            ..light
        };
        assert!(relights_clouds(on, Some((&lit, dim))));
        let white = GlowLight {
            radiance: [0.04, 0.04, 0.04],
            ..light
        };
        assert!(relights_clouds(on, Some((&lit, white))));
        let moved = GlowLight {
            offset: [0.0, 0.0],
            ..light
        };
        assert!(relights_clouds(on, Some((&lit, moved))));
        // A new map, though the header is the same (a cover's rewrite): the field is laid again.
        let rewritten = GroundGlow {
            version: 2,
            ..map(1.0)
        };
        assert!(relights_clouds(on, Some((&rewritten, light))));
        // Burning neither before nor after (by day), whatever the map: nothing to march again for.
        let day = GlowLight {
            radiance: [0.0; 3],
            ..light
        };
        assert!(!relights_clouds(Some((&lit, day)), Some((&rewritten, day))));
        assert!(!relights_clouds(Some((&lit, day)), None));
    }

    #[test]
    fn the_air_lays_it_again_only_when_it_moves() {
        let air = [4e-5, 250.0, 2.8e-5, 2000.0];
        assert!(!air_moved(air, [4.1e-5, 250.0, 2.8e-5, 2000.0]));
        assert!(air_moved(air, [8e-5, 250.0, 2.8e-5, 2000.0]));
        assert!(!air_moved([0.0; 4], [0.0; 4]));
    }
}
