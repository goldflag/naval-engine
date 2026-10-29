//! The warp trial ([`crate::Trial::warp`]): still puffs made to roll in the sprite shaders (`shaders/warp.wgsl`).
//!
//! The CPU half is small. A puff's picture is pulled along a flow fixed in the puff, and how far it has been pulled
//! is the puff's own: [`roll`], from how much it has swollen and how old it is, so a fireball boils while it bursts
//! and settles as it stops growing, a replay rolls the same and a redraw of a held frame does not move.
//! [`axis`] is the direction a puff was thrown or rises in, which its thermal rolls about. [`flow_texture`] is the
//! map of eddies the shaders add to that thermal, made at load.
//!
//! The pools publish both whether or not the trial is on (a logarithm per puff); the shaders read them only when it is.

use glam::DVec3;

use crate::textures::Image2d;

/// Radii of roll for each e-fold a puff swells by: a fireball that swells fourfold in its first half second rolls
/// 0.4 of its radius in that time, most of a cycle of the shader's layers. Faster and a fireball's lobes change
/// every few frames, which reads as flicker.
pub const ROLL_PER_GROWTH: f64 = 0.3;
/// Radii of roll a second once a puff has stopped swelling: smoke that hangs keeps turning over, a cycle of the
/// shader's layers in three or four seconds.
pub const ROLL_PER_SECOND: f64 = 0.14;

/// How far a puff's picture has rolled along its flow, in puff radii where the flow is 1: `size` now against the
/// size it was `born` with, at `age` seconds. A function of the puff's own state alone.
pub fn roll(size: f64, born: f64, age: f64) -> f32 {
    let swollen = (size / born.max(1e-3)).max(1.0).ln();
    (ROLL_PER_GROWTH * swollen + ROLL_PER_SECOND * age.max(0.0)) as f32
}

/// The direction a pool's puff rolls about: the way it was thrown. Drag scales a particle's velocity (less the
/// terminal velocity gravity gives it) without turning it, so this is the launch's direction for as long as the
/// velocity can be told from that terminal one; a puff thrown nowhere rises. The pool takes it at the particle's
/// first step and keeps it (`EffectParticle::roll_axis`): seconds on, a fireball's velocity is all rounding.
pub fn axis(velocity: DVec3, gravity: f64, drag: f64) -> DVec3 {
    let terminal = if drag > 0.0001 { gravity / drag } else { 0.0 };
    let thrown = DVec3::new(velocity.x, velocity.y + terminal, velocity.z);
    let speed = thrown.length();
    if speed > 1e-9 {
        thrown / speed
    } else {
        DVec3::Y
    }
}

/// A gas puff's `warp`: its roll and the axis it rolls about.
pub fn gas(roll: f32, axis: DVec3) -> [f32; 4] {
    [roll, axis.x as f32, axis.y as f32, axis.z as f32]
}

/// A sprite's one spare number: its roll, with its seed (the pools' 0 … 100) above it in whole sixteens, one of 256,
/// which `roll_sprite` in `shaders/warp.wgsl` takes apart again. A sprite has no seed of its own, and its turn drifts.
pub fn sprite(roll: f32, seed: f64) -> f32 {
    roll.clamp(0.0, 15.99) + 16.0 * (seed.rem_euclid(1.0) * 256.0).floor() as f32
}

/// Edge of the eddy map in texels.
pub const FLOW_SIZE: usize = 64;

/// The eddies: a tiling 64² map of divergence-free flow, the curl of a potential summed from a few waves that fit the
/// tile (so it repeats without a seam and its curl is exact). R and G hold the flow's x and y (0.5 is still, the
/// fastest texel a full 0 or 1), B the potential, A a second, unrelated potential. No mips: a puff reads a third of
/// the map across its whole width.
pub fn flow_texture() -> Image2d {
    // Whole waves across the tile, the long ones strongest.
    const WAVES: [(i32, i32); 12] = [
        (1, 0),
        (0, 1),
        (1, 1),
        (1, -1),
        (2, 1),
        (1, 2),
        (2, -1),
        (-1, 2),
        (2, 2),
        (3, 1),
        (1, -3),
        (3, -2),
    ];
    let mut random = crate::random::Lcg::new(1905);
    let mut field = |waves: &[(i32, i32)]| -> Vec<(f64, f64, f64, f64)> {
        waves
            .iter()
            .map(|&(kx, ky)| {
                let (kx, ky) = (f64::from(kx), f64::from(ky));
                let amplitude = (0.6 + random.next() * 0.8) / kx.hypot(ky).powf(1.6);
                (kx, ky, amplitude, random.next() * std::f64::consts::TAU)
            })
            .collect()
    };
    let flow = field(&WAVES);
    let other = field(&WAVES[..8]);
    let tau = std::f64::consts::TAU;
    // The potential and its curl (∂ψ/∂y, −∂ψ/∂x) at a point of the unit tile.
    let sample = |waves: &[(f64, f64, f64, f64)], x: f64, y: f64| {
        let (mut potential, mut u, mut v) = (0.0, 0.0, 0.0);
        for &(kx, ky, amplitude, phase) in waves {
            let angle = tau * (kx * x + ky * y) + phase;
            potential += amplitude * angle.sin();
            u += amplitude * ky * angle.cos();
            v -= amplitude * kx * angle.cos();
        }
        (potential, u, v)
    };
    let texel = |i: usize| (i as f64 + 0.5) / FLOW_SIZE as f64;
    let points: Vec<_> = (0..FLOW_SIZE * FLOW_SIZE)
        .map(|i| {
            let (x, y) = (texel(i % FLOW_SIZE), texel(i / FLOW_SIZE));
            let (potential, u, v) = sample(&flow, x, y);
            (potential, u, v, sample(&other, x, y).0)
        })
        .collect();
    let fastest = points.iter().map(|p| p.1.hypot(p.2)).fold(1e-9, f64::max);
    let reach = |pick: fn(&(f64, f64, f64, f64)) -> f64| {
        points.iter().map(|p| pick(p).abs()).fold(1e-9, f64::max)
    };
    let (deepest, other_deepest) = (reach(|p| p.0), reach(|p| p.3));
    let byte = |signed: f64| ((signed * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut pixels = Vec::with_capacity(points.len() * 4);
    for p in &points {
        pixels.extend([
            byte(p.1 / fastest),
            byte(p.2 / fastest),
            byte(p.0 / deepest),
            byte(p.3 / other_deepest),
        ]);
    }
    Image2d {
        size: FLOW_SIZE as u32,
        levels: vec![pixels],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_puff_rolls_fastest_while_it_swells_and_never_backwards() {
        // A fireball lobe: 6 m swelling by 120 m/s that decays at 5.5 /s.
        let size = |age: f64| 6.0 + 120.0 * (1.0 - (-age * 5.5f64).exp()) / 5.5 + age;
        let at = |age: f64| roll(size(age), 6.0, age);
        assert_eq!(at(0.0), 0.0);
        let early = at(0.3) - at(0.0);
        let late = at(5.3) - at(5.0);
        assert!(early > late * 8.0, "{early} against {late}");
        let mut last = 0.0;
        for step in 1..600 {
            let now = at(f64::from(step) / 60.0);
            assert!(now >= last);
            last = now;
        }
        // The same puff at the same age has rolled the same.
        assert_eq!(at(2.5), at(2.5));
    }

    #[test]
    fn a_sprite_carries_its_roll_and_its_seed_in_one_number() {
        // As `roll_sprite` takes them apart.
        let packed = sprite(1.375, 61.803_398_875);
        let sixteens = (packed / 16.0).floor();
        assert_eq!(packed - sixteens * 16.0, 1.375);
        assert_eq!(sixteens, (0.803_398_875f64 * 256.0).floor() as f32);
        assert!(sprite(40.0, 0.0) < 16.0);
    }

    #[test]
    fn the_axis_is_the_throw_while_drag_slows_it() {
        // As `ParticlePool::advance` integrates a dragged particle under gravity.
        let (drag, gravity) = (7.0, -1.5);
        let terminal = gravity / drag;
        let thrown = DVec3::new(180.0, 12.0, -40.0);
        let first = axis(thrown, gravity, drag);
        let mut velocity = thrown;
        for _ in 0..90 {
            let decay = (-drag / 60.0f64).exp();
            velocity.x *= decay;
            velocity.z *= decay;
            velocity.y = (velocity.y + terminal) * decay - terminal;
        }
        assert!(velocity.length() < 1.0);
        assert!(axis(velocity, gravity, drag).dot(first) > 0.999_999);
        assert_eq!(axis(DVec3::ZERO, 0.0, 0.0), DVec3::Y);
    }

    #[test]
    fn a_pool_keeps_a_puffs_axis_from_its_first_step() {
        use crate::particles::{ParticlePool, PoolKind};
        let mut pool = ParticlePool::new(4, PoolKind::Gas);
        let p = pool.emit(DVec3::ZERO, None);
        p.velocity = DVec3::new(180.0, 12.0, -40.0);
        p.drag = 7.0;
        p.gravity = -1.5;
        p.life = 12.0;
        let thrown = axis(p.velocity, p.gravity, p.drag);
        // Ten seconds on its velocity is rounding error about the terminal one; the axis is still the throw's.
        for _ in 0..600 {
            pool.advance(1.0 / 60.0, DVec3::ZERO);
        }
        assert_eq!(pool.particles()[0].roll_axis, thrown);
    }

    #[test]
    fn the_eddies_tile_and_neither_gather_nor_spread() {
        let image = flow_texture();
        let n = FLOW_SIZE;
        assert_eq!(image.levels[0].len(), n * n * 4);
        let at = |x: usize, y: usize, c: usize| {
            f64::from(image.levels[0][((y % n) * n + x % n) * 4 + c]) / 255.0 * 2.0 - 1.0
        };
        let mut fastest: f64 = 0.0;
        let mut divergence: f64 = 0.0;
        let mut drift = (0.0, 0.0);
        for y in 0..n {
            for x in 0..n {
                let (u, v) = (at(x, y, 0), at(x, y, 1));
                fastest = fastest.max(u.hypot(v));
                drift = (drift.0 + u, drift.1 + v);
                // Central differences across the wrap: a tiling curl has none, to the bytes' rounding.
                let spread = (at(x + 1, y, 0) - at(x + n - 1, y, 0)) / 2.0
                    + (at(x, y + 1, 1) - at(x, y + n - 1, 1)) / 2.0;
                divergence = divergence.max(spread.abs());
            }
        }
        assert!(fastest > 0.98 && fastest < 1.01, "{fastest}");
        assert!(divergence < 0.02, "{divergence}");
        // No net current: the eddies turn the picture over, they do not carry it off.
        let texels = (n * n) as f64;
        assert!(drift.0.abs() / texels < 0.01 && drift.1.abs() / texels < 0.01);
    }
}
