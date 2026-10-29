//! The land's open sky on the CPU: what `shaders/sky_bake.wgsl` bakes on the GPU, for tests and probes.
//!
//! Ports `SkyBake` (the web game's `terrain/TerrainBake.ts`) **as the game's generated WGSL runs it**: its two nested
//! TSL `Loop`s
//! both name their counter `i`, the inner one shadows the outer, and the direction is emitted inside the inner loop, so
//! step `s` walks direction `s` (a spiral of 20 samples) and the 16 outer passes repeat it. [`open_sky_as_written`] is the
//! walk `TerrainBake.ts` reads as (each of 16 directions out in 20 steps), which the web does not run; the difference is
//! the valley and gully darkness (`native/docs/terrain/shadows.md`, "The open sky"). `web/open-sky-bake-check.ts` dumps
//! the web's WGSL and reads its bake back.

use crate::field::Grid;

/// Directions, steps, the first step and its growth (`SKY_DIRECTIONS`, `SKY_STEPS`, `SKY_FIRST_STEP_M`, `SKY_STEP_GROWTH`).
pub const SKY_DIRECTIONS: u32 = 16;
pub const SKY_STEPS: u32 = 20;
pub const SKY_FIRST_STEP_M: f32 = 40.0;
pub const SKY_STEP_GROWTH: f32 = 1.25;

/// The field the bake reads: heights in metres and half-float gradients (∂h/∂x, ∂h/∂z) per metre on `grid`.
#[derive(Clone, Copy)]
pub struct BakeInput<'a> {
    pub grid: Grid,
    pub heights: &'a [f32],
    pub gradients: &'a [u16],
}

impl BakeInput<'_> {
    fn height(&self, x: i64, z: i64) -> f32 {
        self.heights[(z * i64::from(self.grid.columns) + x) as usize]
    }

    /// The sky open over field sample (`x`, `z`) when step `s` of direction `i` walks direction `walk(i, s)`, in f32 as
    /// the GPU computes it.
    fn open(&self, x: i64, z: i64, walk: impl Fn(u32, u32) -> u32) -> f32 {
        let cell = self.grid.cell as f32;
        let (last_x, last_z) = ((self.grid.columns - 1) as f32, (self.grid.rows - 1) as f32);
        let base = self.height(x, z);
        let k = (z * i64::from(self.grid.columns) + x) as usize * 2;
        let slope = (
            half::f16::from_bits(self.gradients[k]).to_f32(),
            half::f16::from_bits(self.gradients[k + 1]).to_f32(),
        );
        let mut open = 0.0;
        for i in 0..SKY_DIRECTIONS {
            let mut horizon = 0.0f32;
            let mut distance = SKY_FIRST_STEP_M;
            for s in 0..SKY_STEPS {
                let angle =
                    walk(i, s) as f32 * (std::f32::consts::TAU / SKY_DIRECTIONS as f32) + 0.2;
                let (dx, dz) = (angle.cos(), angle.sin());
                let px = (x as f32 + dx * (distance / cell))
                    .round()
                    .clamp(0.0, last_x);
                let pz = (z as f32 + dz * (distance / cell))
                    .round()
                    .clamp(0.0, last_z);
                let height = self.height(px as i64, pz as i64);
                horizon = horizon.max((height - base) / distance - (slope.0 * dx + slope.1 * dz));
                distance *= SKY_STEP_GROWTH;
            }
            open += 1.0 / (horizon * horizon + 1.0);
        }
        open / SKY_DIRECTIONS as f32
    }
}

/// The open sky over field sample (`x`, `z`), 0–1, as the web bakes it (and `sky_bake.wgsl` does).
pub fn open_sky(input: &BakeInput<'_>, x: u32, z: u32) -> f32 {
    input.open(i64::from(x), i64::from(z), |_, s| s)
}

/// The open sky over field sample (`x`, `z`) as `TerrainBake.ts` reads: each of the 16 directions walked in 20 steps.
pub fn open_sky_as_written(input: &BakeInput<'_>, x: u32, z: u32) -> f32 {
    input.open(i64::from(x), i64::from(z), |i, _| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::half_bits;

    const COLUMNS: u32 = 255;
    const CELL: f64 = 20.0;

    /// The synthetic field of `web/open-sky-bake.ts`: long sines, a ridge line and a pit; gradients by central differences.
    fn field() -> (Vec<f32>, Vec<u16>) {
        let n = COLUMNS as usize;
        let mut heights = vec![0.0f32; n * n];
        for z in 0..n {
            for x in 0..n {
                let (fx, fz) = (x as f64, z as f64);
                let ridge = 180.0 * (-((fx - 150.0) / 12.0).powi(2)).exp();
                let pit = -120.0 * (-((fx - 70.0).powi(2) + (fz - 90.0).powi(2)) / 400.0).exp();
                heights[z * n + x] = (60.0 * (fx / 9.0).sin() * (fz / 13.0).cos()
                    + 40.0 * ((fx + 2.0 * fz) / 23.0).sin()
                    + ridge
                    + pit
                    + 200.0) as f32;
            }
        }
        let h = |x: i64, z: i64| {
            f64::from(
                heights
                    [(z.clamp(0, n as i64 - 1) as usize) * n + x.clamp(0, n as i64 - 1) as usize],
            )
        };
        let mut gradients = vec![0u16; n * n * 2];
        for z in 0..n as i64 {
            for x in 0..n as i64 {
                let k = (z as usize * n + x as usize) * 2;
                gradients[k] = half_bits((h(x + 1, z) - h(x - 1, z)) / (2.0 * CELL));
                gradients[k + 1] = half_bits((h(x, z + 1) - h(x, z - 1)) / (2.0 * CELL));
            }
        }
        (heights, gradients)
    }

    /// The web's own bake, read back at five texels of the attribute grid (`open-sky-bake-check.ts`, 2026-09-30): 8-bit
    /// values. The model as the web runs it lands on each within rounding; the walk as written misses three.
    #[test]
    fn the_model_is_what_the_web_bakes() {
        let (heights, gradients) = field();
        let input = BakeInput {
            grid: Grid {
                columns: COLUMNS,
                rows: COLUMNS,
                cell: CELL,
            },
            heights: &heights,
            gradients: &gradients,
        };
        let web = [
            ((10, 10), 231u8),
            ((35, 45), 244),
            ((60, 60), 222),
            ((75, 30), 255),
            ((100, 100), 253),
        ];
        let mut written_misses = 0;
        for ((x, z), byte) in web {
            let baked = f32::from(byte) / 255.0;
            let ours = open_sky(&input, x * 2, z * 2);
            assert!(
                (ours - baked).abs() <= 1.0 / 255.0,
                "({x}, {z}): {ours} vs the web's {baked}"
            );
            if (open_sky_as_written(&input, x * 2, z * 2) - baked).abs() > 2.0 / 255.0 {
                written_misses += 1;
            }
        }
        assert_eq!(written_misses, 3);
    }
}
