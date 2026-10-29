//! Shell splashes and the other water the combat effects throw (the water events of `CombatEffects.emit` in the web
//! game's `CombatEffects.ts`): each throws a column of water ([`SplashColumns`]: the column, the veil of mist it sheds,
//! the crown at its foot and the surge its falling water raises, marched through on the GPU), and a crashing aircraft a
//! fan of drops besides. Each event draws from its own seeded stream ([`Lcg::for_event`], the web game's
//! `randomFor`), so the same event always throws the same water. The patch of foam a splash leaves on the sea is the
//! ocean's (the trail foam's impacts).

use glam::DVec3;

use crate::columns::{SplashColumns, size_of};
use crate::fixture::{WaterEvent, WaterEventKind};
use naval_effects::CameraView;
use naval_effects::instances::WaterSprite;
use naval_effects::math::Rgb;
use naval_effects::particles::{ParticlePool, PoolKind};
use naval_effects::random::Lcg;

/// A torpedo entering the sea throws a narrow splash, about a tenth of a heavy shell column's height.
pub const TORPEDO_ENTRY_SCALE: f64 = 0.025;
/// Equivalent shell calibre for the small foam ring the entry leaves on the ocean.
pub const TORPEDO_ENTRY_CALIBER: f64 = 0.38 * TORPEDO_ENTRY_SCALE;

/// An sRGB channel to linear, as three's `Color('#hex')` converts it.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c < 0.04045 {
        c * 0.077_399_380_804_953_56
    } else {
        (c * 0.947_867_298_578_199 + 0.052_132_701_421_800_95).powf(2.4)
    }
}

/// A `#rrggbb` colour in linear RGB.
pub fn hex(rgb: u32) -> DVec3 {
    let channel = |shift: u32| srgb_to_linear(f64::from((rgb >> shift) & 255) / 255.0);
    DVec3::new(channel(16), channel(8), channel(0))
}

/// `WATER` of CombatEffects.ts and BowSpray.ts: `#e7f2f1`.
pub fn water_color() -> Rgb {
    let c = hex(0xe7f2f1);
    Rgb::new(c.x, c.y, c.z)
}

/// The splash water of every combat event: its columns, and the drops a crashing aircraft throws.
pub struct Splashes {
    pub columns: SplashColumns,
    /// The drops a crashing aircraft's wreckage throws forward.
    pub spray: ParticlePool,
    /// The drops' sprites as last published, back to front.
    pub drops: Vec<WaterSprite>,
    sequence: u64,
}

impl Default for Splashes {
    fn default() -> Self {
        Self::new()
    }
}

impl Splashes {
    /// Room for a fleet action: a 3 v 3 battleship action throws two or three heavy splashes a second, each standing
    /// up to 22 s.
    pub fn new() -> Self {
        Self {
            columns: SplashColumns::new(384),
            spray: ParticlePool::new(
                1024,
                PoolKind::Water {
                    fine: true,
                    offscreen: false,
                },
            ),
            drops: Vec::new(),
            sequence: 0,
        }
    }

    /// Fraction of particles emitted (the combat effects density setting): the drops only, a column is one draw.
    pub fn set_density(&mut self, density: f64) {
        self.spray.density = density;
    }

    /// Age everything by `dt` before the frame's events emit.
    pub fn advance(&mut self, dt: f64, wind: DVec3) {
        self.spray.advance(dt, wind);
        self.columns.advance(dt, wind);
    }

    /// Throw the water of every event newer than the last one seen.
    pub fn emit(&mut self, events: &[WaterEvent]) {
        for event in events {
            if event.sequence <= self.sequence {
                continue;
            }
            self.sequence = event.sequence;
            self.event(event);
        }
    }

    fn event(&mut self, event: &WaterEvent) {
        let mut random = Lcg::for_event(event.sequence, event.shell_id);
        let mut position = DVec3::from(event.position);
        let mut direction =
            DVec3::from(event.velocity.unwrap_or([0.0, -1.0, 0.0])).normalize_or(DVec3::NEG_Y);
        match event.kind {
            WaterEventKind::TorpedoEntry => {
                position.y = 0.0;
                self.splash(position, direction, TORPEDO_ENTRY_SCALE, &mut random);
            }
            WaterEventKind::AircraftCrash => {
                let surface = DVec3::new(position.x, 0.0, position.z);
                self.splash(surface, direction, 1.3, &mut random);
                // Forward-moving wreckage throws a broad low fan beyond the main water column.
                direction.y = 0.0;
                direction = direction.normalize_or(DVec3::X);
                let water = water_color();
                position.y = 0.35;
                for _ in 0..30 {
                    let p = self.spray.emit(position, None);
                    p.velocity = direction * (8.0 + random.next() * 22.0);
                    p.velocity.x += (random.next() - 0.5) * 18.0;
                    p.velocity.z += (random.next() - 0.5) * 18.0;
                    p.velocity.y = 5.0 + random.next() * 14.0;
                    p.size = 1.0 + random.next() * 2.0;
                    p.growth = 1.5;
                    p.life = 3.0;
                    p.gravity = 9.81;
                    p.drag = 0.3;
                    p.waterline = true;
                    p.opacity = 0.7;
                    p.color = water;
                }
            }
            WaterEventKind::DepthChargeBlast | WaterEventKind::DepthChargeSplash => {
                position.y = 0.0;
                let scale = if event.kind == WaterEventKind::DepthChargeBlast {
                    1.35
                } else {
                    0.3
                };
                self.splash(position, direction, scale, &mut random);
            }
            WaterEventKind::TorpedoHit => {
                position.y = 0.0;
                self.splash(position, direction, 1.7, &mut random);
            }
            WaterEventKind::Splash => {
                let scale = (event.caliber_m.unwrap_or(0.38) / 0.38).clamp(0.035, 1.7);
                self.splash(position, direction, scale, &mut random);
            }
        }
    }

    /// One column at `at` (its water level), `scale` (1 for 38 cm), the shell's `direction`. No two columns of one
    /// calibre stand quite the same height.
    fn splash(&mut self, at: DVec3, direction: DVec3, scale: f64, random: &mut Lcg) {
        let seed = random.next();
        let size = size_of(scale) * (0.9 + 0.2 * random.next());
        self.columns.emit(at, size, direction, seed);
    }

    /// Build the frame's columns and sprites, after every emission.
    pub fn publish(&mut self, camera: &CameraView) {
        self.columns.publish(camera);
        self.drops.clear();
        self.spray.publish_water_into(camera, None, &mut self.drops);
    }

    pub fn reset(&mut self) {
        self.columns.reset();
        self.spray.reset();
        self.drops.clear();
        self.sequence = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splash(sequence: u64, shell_id: i64) -> WaterEvent {
        WaterEvent {
            sequence,
            kind: WaterEventKind::Splash,
            position: [10.0, 0.0, -20.0],
            velocity: Some([-300.0, -450.0, 0.0]),
            caliber_m: Some(0.38),
            shell_id: Some(shell_id),
        }
    }

    #[test]
    fn a_torpedo_entry_throws_a_small_splash_once_at_the_surface() {
        let mut entry = splash(4, 100);
        entry.kind = WaterEventKind::TorpedoEntry;
        entry.position[1] = -2.0;
        entry.velocity = Some([0.0, -20.0, -60.0]);
        let mut small = Splashes::new();
        let mut heavy = Splashes::new();
        small.emit(&[entry.clone()]);
        heavy.emit(&[splash(5, 101)]);
        let (a, b) = (&small.columns.columns()[0], &heavy.columns.columns()[0]);
        assert_eq!(a.origin.y, 0.0);
        assert!(a.size < b.size * 0.2, "{} {}", a.size, b.size);
        let before = small.columns.columns().to_vec();
        small.emit(&[entry]);
        assert_eq!(small.columns.columns(), before);
    }

    #[test]
    fn the_same_event_throws_the_same_water() {
        let mut a = Splashes::new();
        let mut b = Splashes::new();
        a.emit(&[splash(4, 1_000_001)]);
        b.emit(&[splash(4, 1_000_001)]);
        assert_eq!(a.columns.columns(), b.columns.columns());
        let mut c = Splashes::new();
        c.emit(&[splash(5, 1_000_001)]);
        assert_ne!(a.columns.columns(), c.columns.columns());
        // One column a splash, leaning along the shell's way.
        assert_eq!(a.columns.columns().len(), 1);
        assert!(a.columns.columns()[0].lean.x < 0.0);
        // An event seen before is not thrown again.
        a.emit(&[splash(4, 1_000_001)]);
        assert_eq!(a.columns.columns().len(), 1);
    }
}
