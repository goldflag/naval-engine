//! Trial look `flipbook`: what the blasts emit when their gas is drawn from baked simulations (`flipbook.rs`).
//!
//! A simulated puff carries its own structure (a fireball heaving up on a stem, its skin of soot, the smoke it
//! leaves), its own growth and its own turn from flame to soot, so a blast needs fewer and larger puffs than the
//! piles of round lobes `blasts.rs` builds a fireball from, and its heat is held: the simulation decides where it
//! still burns. `blasts.rs` calls these in place of its own lobes while the trial is on.

use glam::DVec3;

use crate::flipbook::{GROUND_ANCHOR, clip};
use crate::math::Rgb;
use crate::particles::{EffectParticle, ParticlePool, SourceId};
use crate::random::Lcg;

const UP: DVec3 = DVec3::Y;

/// A puff playing a grounded clip, standing on `foot`: `size` is the full-grown tile's edge (m).
fn standing(p: &mut EffectParticle, foot: DVec3, size: f64) {
    p.position = foot + UP * (size * 0.5 * GROUND_ANCHOR);
    p.size = size;
}

/// Heat that outlasts the puff: the simulation's flame decides what burns and when it goes out.
fn held_heat(p: &mut EffectParticle, heat: f64) {
    p.heat = heat;
    p.cooling = p.life * 6.0;
}

/// An HE burst of size `s` at `at`, blown out over `normal`'s side: one simulated blast standing on the plating,
/// two smaller fireballs thrown off it (so it has depth from any side) and the smoke that hangs after.
pub(crate) fn burst(
    volumes: &mut ParticlePool,
    at: DVec3,
    normal: DVec3,
    s: f64,
    source: Option<&SourceId>,
    smoke: Rgb,
    random: &mut Lcg,
) {
    let p = volumes.emit(at, source);
    standing(
        p,
        at + normal * (1.2 * s) - UP * (0.6 * s),
        (19.0 + random.next() * 4.0) * s,
    );
    p.velocity = normal * (1.5 * s);
    p.drag = 1.2;
    p.diffusion = 0.5 * s;
    p.life = 8.5 + random.next() * 1.5;
    held_heat(p, 1.95);
    p.wind = 0.6;
    p.density = 5.5;
    p.opacity = 0.97;
    p.color = smoke.scale(0.95 + random.next() * 0.2);
    p.flipbook = clip::BLAST;
    for _ in 0..2 {
        let p = volumes.emit(at, source);
        let yaw = random.next() * std::f64::consts::TAU;
        let y = random.next() * 1.2 - 0.2;
        let r = (1.0 - (y * y).min(1.0)).sqrt();
        let point = (DVec3::new(yaw.cos() * r, y, yaw.sin() * r) + normal * 0.9).normalize_or(UP);
        p.position += point * ((1.5 + random.next() * 2.0) * s);
        p.velocity = point * ((9.0 + random.next() * 9.0) * s);
        p.velocity.y += 2.0 * s;
        p.drag = 3.0;
        p.size = (9.0 + random.next() * 3.0) * s;
        p.diffusion = 0.5 * s;
        p.life = 6.5 + random.next() * 2.0;
        held_heat(p, 1.9);
        p.gravity = -1.2;
        p.wind = 0.6;
        p.density = 5.0;
        p.opacity = 0.96;
        p.color = smoke.scale(0.85 + random.next() * 0.3);
        p.flipbook = clip::FIREBALL;
    }
    // The smoke that hangs over the hit after the blast's own has thinned.
    for i in 0..3 {
        let p = volumes.emit(at, source);
        p.age = -(0.5 + f64::from(i) * 0.45 + random.next() * 0.3);
        p.position += normal * (2.0 * s) + UP * ((3.0 + random.next() * 4.0) * s);
        p.position += DVec3::new(random.next() - 0.5, 0.0, random.next() - 0.5) * (5.0 * s);
        p.velocity = DVec3::new(
            (random.next() - 0.5) * 3.0,
            2.5 + random.next() * 3.0,
            (random.next() - 0.5) * 3.0,
        ) * s;
        p.drag = 1.0;
        p.size = (9.0 + random.next() * 4.0) * s;
        p.diffusion = 1.0 * s;
        p.life = 9.0 + random.next() * 3.0;
        p.fade_in = 0.6;
        p.dissipation_time = 5.0;
        p.gravity = -0.9;
        p.wind = 0.8;
        p.density = 4.2;
        p.opacity = 0.9;
        p.color = smoke.scale(1.05 + random.next() * 0.2);
        p.flipbook = clip::SMOKE;
    }
}

/// A burst tearing up through the deck at `at`, of size `size`: one simulated blast standing on the deck in place
/// of six lobes.
pub(crate) fn eruption(
    volumes: &mut ParticlePool,
    at: DVec3,
    size: f64,
    source: Option<&SourceId>,
    smoke: Rgb,
    random: &mut Lcg,
) {
    let p = volumes.emit(at, source);
    standing(
        p,
        at - UP * (0.5 * size),
        (17.0 + random.next() * 4.0) * size,
    );
    p.diffusion = 0.5 * size;
    p.life = 8.0 + random.next() * 1.5;
    held_heat(p, 1.95);
    p.wind = 0.6;
    p.density = 5.5;
    p.opacity = 0.97;
    p.color = smoke.scale(0.9 + random.next() * 0.25);
    p.flipbook = clip::BLAST;
}

/// A hot lobe as `blasts.rs` set it (a turret's secondary flash, a jet out of a shell hole): the simulated fireball
/// in its place. The frame's own size starts at half the puff's, so the puff is made larger to reach as far.
pub(crate) fn fireball(p: &mut EffectParticle) {
    p.size *= 1.5;
    p.growth *= 1.25;
    p.density = 4.2;
    p.flipbook = clip::FIREBALL;
}

/// A billow of smoke as `blasts.rs` set it (the gunsmoke bank, what pours out of a hole): simulated smoke rolling
/// and thinning.
pub(crate) fn smoke(p: &mut EffectParticle) {
    p.size *= 1.2;
    p.density *= 1.6;
    p.flipbook = clip::SMOKE;
}

/// A magazine's fireball of size `s` on `base`: three simulated blasts side by side, slow as so large a burst is,
/// in place of sixteen lobes.
pub(crate) fn magazine_fireball(
    volumes: &mut ParticlePool,
    base: DVec3,
    s: f64,
    source: Option<&SourceId>,
    smoke: Rgb,
    random: &mut Lcg,
) {
    let yaw = random.next() * std::f64::consts::TAU;
    for (i, (size, out, delay)) in [(230.0, 0.0, 0.0), (150.0, 30.0, 0.25), (125.0, 34.0, 0.55)]
        .into_iter()
        .enumerate()
    {
        let angle = yaw + i as f64 * 2.3;
        let foot = base + DVec3::new(angle.cos(), 0.0, angle.sin()) * (out * s) - UP * (3.0 * s);
        let p = volumes.emit(base, source);
        standing(p, foot, size * s * (0.92 + random.next() * 0.16));
        p.age = -delay;
        p.diffusion = 0.6 * s;
        p.life = 46.0 + random.next() * 10.0;
        held_heat(p, 2.0);
        p.wind = 0.5;
        p.density = 6.0;
        p.opacity = 0.97;
        p.color = smoke.scale(0.85 + random.next() * 0.25);
        p.flipbook = clip::BLAST_SLOW;
    }
}

/// A billow of a magazine's column or cap: simulated smoke (its flame, where it has heat).
pub(crate) fn magazine_billow(p: &mut EffectParticle) {
    p.size *= 1.25;
    p.density *= 1.7;
    p.flipbook = clip::SMOKE;
}

/// A puff of one of a magazine's secondary explosions: a simulated fireball.
pub(crate) fn magazine_secondary(p: &mut EffectParticle) {
    p.size *= 1.5;
    p.density *= 1.7;
    held_heat(p, 1.9);
    p.flipbook = clip::FIREBALL;
}
