//! Trial look `flipbook`: a burning place's flames drawn from a baked fire simulation (`flipbook::clip::FIRE`), in
//! place of the flame tongues and rolling flame bodies `fires.rs` emits.
//!
//! The simulation is a fuel fire: a turbulent body of flame, hotter at its foot and core, darker where it thins and
//! cools, its tongues breaking off into the soot that rolls up from their tops. A fire draws a few large sprites of
//! it standing on its roof or hatch, each at its own place in the loop and its own pace, every other one mirrored, so
//! they overlap into one broad body no two parts of which move together. They are not particles: each is worked out
//! from the fire and the clock every frame, so a fire grows and dies down without a sprite being born or lost, and
//! the sprites of two fires never share a phase. The sprite stands upright, turns about the vertical to face the lens
//! and leans with the air over the fire (`flipbook_stand` in `shaders/flipbook.wgsl`); its soot thins out toward the
//! top of its tile, over the fire's own smoke column, which is drawn behind it.

use glam::DVec3;

use crate::camera::CameraView;
use crate::flipbook::{FIRE_LOOP_SECONDS, clip};
use crate::instances::GasPuff;
use crate::math::{Rgb, ShipPose, smoothstep};

/// Sprites a fire draws at most.
pub(crate) const FLAMES_PER_FIRE: usize = 5;
/// The share of the tile's height the simulation's flame reaches, most of the time.
pub(crate) const FLAME_SHARE: f64 = 0.54;
/// The share of the tile's height its foot stands below where the fire is.
pub(crate) const SINK: f64 = 0.04;
/// The flame's tilt from the vertical in the air past it, the pool-fire correlation of the American Gas Association
/// (Thomas; the SFPE handbook's): upright while the air is slower than the fire's own characteristic velocity
/// (g·m″·D/ρₐ)^⅓, past it cos θ = u*^(−½), u* the air's speed over that. Fuel oil burns at some 0.05 kg/m²s
/// (`BURNING_RATE`) in air of 1.2 kg/m³: a burning roof 15 m across stands upright in 1.8 m/s, leans 39° in 3 m/s and
/// would lean 63° in 9.
const BURNING_RATE: f64 = 0.05;
const AIR_DENSITY: f64 = 1.2;
/// At most 50° (1.2 m downwind per metre up), short of the correlation's 60° and more in a fresh wind: the flame is a
/// card sheared along its lean (`flipbook_stand`), and past some 55° its tongues streak into a flat sheet along the
/// deck (filmed 2026-10-25 at 50, 55 and 63°, `flames`). A big fire reaches it in 5 m/s of air, a hatch's in 2.6.
const MAX_TILT: f64 = 50.0 * std::f64::consts::PI / 180.0;

/// The flame's lean (metres downwind per metre up, horizontal) in `air` (m/s, horizontal: the wind at its height less
/// her way) over a fire `width` metres across.
fn lean_in(air: DVec3, width: f64) -> DVec3 {
    let characteristic = (9.81 * BURNING_RATE * width.max(1.0) / AIR_DENSITY).cbrt();
    let speed = air.length() / characteristic;
    if speed <= 1.0 {
        return DVec3::ZERO;
    }
    let tilt = speed.powf(-0.5).acos().min(MAX_TILT);
    air.normalize_or_zero() * tilt.tan()
}

/// A burning place, as its flames read it.
pub(crate) struct Fire<'a> {
    /// The hull and which of her fires this is: what its sprites' places and phases are drawn from.
    pub ship: &'a str,
    pub key: u64,
    pub pose: ShipPose,
    pub velocity: DVec3,
    /// The middle of the roof or hatch in the hull's frame, its world heading and half extents (m).
    pub local: DVec3,
    pub heading: f64,
    pub half_x: f64,
    pub half_z: f64,
    pub scale: f64,
    pub mount: bool,
    /// The fire's eased intensity (it eases to nothing after the fire goes out).
    pub intensity: f64,
    pub fought: bool,
    pub growing: bool,
    /// Metres from the lens.
    pub distance: f64,
}

/// The scene the flames stand in.
pub(crate) struct Air {
    /// The fires' clock (s).
    pub time: f64,
    /// The smoke's drift (`EffectLighting::wind`): the wind ten metres up.
    pub wind: DVec3,
}

/// A number in 0–1 from a fire's identity, a sprite and a draw: the same every frame.
fn draw(seed: u64, sprite: usize, n: u64) -> f64 {
    // SplitMix64's finalizer.
    let mut z = seed
        .wrapping_add((sprite as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(n.wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

fn identity(fire: &Fire<'_>) -> u64 {
    // FNV-1a over the hull's id, then the fire's key.
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    for byte in fire.ship.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash ^ fire.key.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The fire's sprites, each with its distance from the lens (the caller sorts every fire's back to front).
pub(crate) fn flames(fire: &Fire<'_>, air: &Air, out: &mut Vec<(f64, GasPuff)>) {
    let i = fire.intensity;
    // A hatch shows flame only once the fire below is fierce.
    let vent = if fire.mount {
        1.0
    } else {
        smoothstep(i, 0.3, 0.65)
    };
    if i <= 0.02 || vent <= 0.02 {
        return;
    }
    let seed = identity(fire);
    let stage = if fire.fought {
        0.65
    } else if fire.growing {
        1.1
    } else {
        1.0
    };
    // The flame's height (m) and how many sprites stand side by side: both follow the fire as it grows, and a
    // sprite comes in by growing, never by appearing.
    let height = (2.3 + 5.5 * i) * fire.scale * if fire.mount { 1.0 } else { 0.8 } * stage;
    let far = smoothstep(fire.distance, 1500.0, 5000.0);
    let count = (3.0 + 2.0 * fire.scale * (0.4 + 0.6 * i)) * (1.0 - 0.45 * far);
    // A small fire's flame is quicker than a large one's (time goes as the root of size), by the place's size alone:
    // a pace that followed the intensity would skip through the loop as the fire grew.
    let pace = (1.5 / fire.scale).sqrt().clamp(0.85, 1.6);
    // The air past the flame halfway up it: the wind at that height less her own way, so a following wind takes back
    // the lean her way gives it. It leans as a pool fire as wide as its roof or hatch does (`lean_in`). Until
    // 2026-10-25, 0.6 of the wind and 0.55 of her way over the column's climb, at most 0.9: 37° in 9 m/s at rest.
    let middle = fire.pose.local_to_world(fire.local).y + height * 0.5;
    let past = air.wind * crate::combat::wind_share(middle) - fire.velocity;
    let lean = lean_in(
        DVec3::new(past.x, 0.0, past.z),
        2.0 * (fire.half_x * fire.half_z).sqrt(),
    );
    let (sin, cos) = (fire.heading - fire.pose.heading).sin_cos();
    let turn = draw(seed, 0, 9) * std::f64::consts::TAU;
    // Fuel-oil soot, as the column's: near black with a warm cast. The sprite's light is the atlas's six baked
    // directions, which find more of the sun than the column's own shading does, so its soot is darker to match.
    let shade = ((if fire.growing { 0.075 } else { 0.1 }) + (1.0 - i) * 0.12) * 0.55;
    let albedo = Rgb::new(shade * 1.06, shade, shade * 0.9);
    for sprite in 0..FLAMES_PER_FIRE {
        let weight = (count - sprite as f64).clamp(0.0, 1.0);
        if weight <= 0.02 {
            break;
        }
        let r = |n: u64| draw(seed, sprite, n);
        // The first stands near the middle; the rest round it, a golden angle apart.
        let (ax, az) = if sprite == 0 {
            ((r(0) - 0.5) * 0.5, (r(1) - 0.5) * 0.5)
        } else {
            let angle = turn + sprite as f64 * 2.399_963;
            let reach = 0.3 + 0.3 * r(0);
            (angle.cos() * reach, angle.sin() * reach)
        };
        let (ax, az) = (ax * fire.half_x, az * fire.half_z);
        let foot = fire.pose.local_to_world(DVec3::new(
            fire.local.x + ax * cos - az * sin,
            fire.local.y,
            fire.local.z + ax * sin + az * cos,
        ));
        let variety = if sprite == 0 { 1.12 } else { 0.72 + 0.4 * r(2) };
        let flame = height * variety * (0.55 + 0.45 * weight) * (0.6 + 0.4 * vent);
        let radius = flame / FLAME_SHARE * 0.5;
        // The simulation's burner is a few soft rows above its ground: the sprite's foot goes that far into the roof.
        let foot = foot - DVec3::Y * (radius * 2.0 * SINK);
        // Its own place in the loop and its own pace: no two sprites pulse together.
        let rate = pace * (0.86 + 0.3 * r(3));
        let phase = (air.time * rate / FIRE_LOOP_SECONDS
            + (sprite as f64 + r(4)) / FLAMES_PER_FIRE as f64
            + draw(seed, 0, 8))
        .fract();
        // A slow sway about the lean, each sprite's own.
        let sway = DVec3::new(
            (air.time * (0.9 + 0.5 * r(5)) + r(6) * std::f64::consts::TAU).sin(),
            0.0,
            (air.time * (1.3 + 0.4 * r(7)) + r(8) * std::f64::consts::TAU).sin(),
        ) * 0.05;
        let lean = lean + sway;
        let heat =
            (1.1 + 0.3 * i.sqrt()) * if fire.fought { 0.85 } else { 1.0 } * (0.82 + 0.18 * weight);
        // Every other sprite mirrored (the shader mirrors by `fract(seed × 0.4142) > 0.5`), which one by the fire.
        let mirror = (sprite as f64 + (seed & 1) as f64 + 0.15 + 0.7 * r(10)) * 1.207_15;
        out.push((
            fire.distance,
            GasPuff {
                sphere: [foot.x as f32, foot.y as f32, foot.z as f32, radius as f32],
                state: [0.0, mirror as f32, heat as f32, 4.5],
                tint: [albedo.r as f32, albedo.g as f32, albedo.b as f32, 1.0],
                evolution: [phase as f32, 0.0, 0.0, 0.0],
                quad: [lean.x as f32, 0.0, lean.z as f32, 0.0],
                extra: [
                    (0.97 * weight * vent) as f32,
                    // The flame's light on its own soot from below, in the sun's units (the night's lift takes it).
                    ((0.2 + 0.2 * i) as f32),
                    clip::FIRE as f32,
                    0.0,
                ],
                // The warp trial does not roll a flame.
                warp: [0.0; 4],
            },
        ));
    }
}

/// Every fire's sprites back to front, appended to `out`.
pub(crate) fn publish<'a>(
    fires: impl Iterator<Item = Fire<'a>>,
    air: &Air,
    camera: &CameraView,
    out: &mut Vec<GasPuff>,
) {
    let mut sprites = Vec::new();
    for fire in fires {
        let from = sprites.len();
        flames(&fire, air, &mut sprites);
        // Each sprite by its own foot: one fire's sprites sort among themselves too.
        for (distance, puff) in &mut sprites[from..] {
            let foot = DVec3::new(
                f64::from(puff.sphere[0]),
                f64::from(puff.sphere[1]),
                f64::from(puff.sphere[2]),
            );
            *distance = (foot - camera.position).length_squared();
        }
    }
    sprites.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.extend(sprites.into_iter().map(|(_, puff)| puff));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fire(intensity: f64) -> Fire<'static> {
        Fire {
            ship: "enemy-1",
            key: 3,
            pose: ShipPose {
                x: 100.0,
                y: 0.0,
                z: -50.0,
                heading: 0.4,
                pitch: 0.0,
                roll: 0.0,
            },
            velocity: DVec3::ZERO,
            local: DVec3::new(0.0, 9.0, -40.0),
            heading: 0.4,
            half_x: 4.0,
            half_z: 5.0,
            scale: 1.4,
            mount: true,
            intensity,
            fought: false,
            growing: false,
            distance: 60.0,
        }
    }

    fn sprites(fire: &Fire<'_>, time: f64, wind: DVec3) -> Vec<GasPuff> {
        let mut out = Vec::new();
        flames(fire, &Air { time, wind }, &mut out);
        out.into_iter().map(|(_, puff)| puff).collect()
    }

    #[test]
    fn a_fire_stands_as_a_few_sprites_no_two_in_step() {
        let burning = fire(0.9);
        let now = sprites(&burning, 12.0, DVec3::ZERO);
        assert!((3..=FLAMES_PER_FIRE).contains(&now.len()));
        // Every sprite plays the fire's clip, on the roof, at a place in the loop of its own.
        for puff in &now {
            assert_eq!(puff.extra[2], clip::FIRE as f32);
            assert!(puff.sphere[1] < 9.0 && puff.sphere[1] > 9.0 - 0.1 * puff.sphere[3]);
            assert!((0.0..1.0).contains(&puff.evolution[0]));
        }
        let mut phases: Vec<f32> = now.iter().map(|puff| puff.evolution[0]).collect();
        phases.sort_by(f32::total_cmp);
        assert!(phases.windows(2).all(|pair| pair[1] - pair[0] > 0.02));
        // Mirrored and not, both.
        let mirrored = |puff: &GasPuff| (puff.state[1] * 0.4142).fract() > 0.5;
        assert!(now.iter().any(mirrored) && !now.iter().all(mirrored));
        // A second later each has moved on by its own pace.
        let later = sprites(&burning, 13.0, DVec3::ZERO);
        let mut steps: Vec<f32> = now
            .iter()
            .zip(&later)
            .map(|(a, b)| (b.evolution[0] - a.evolution[0]).rem_euclid(1.0))
            .collect();
        steps.sort_by(f32::total_cmp);
        assert!(steps.windows(2).all(|pair| pair[1] - pair[0] > 1e-4));
        // The same fire at the same moment is the same sprites; another fire's are others.
        assert_eq!(now, sprites(&burning, 12.0, DVec3::ZERO));
        let mut other = fire(0.9);
        other.key = 4;
        assert_ne!(
            now[0].evolution[0],
            sprites(&other, 12.0, DVec3::ZERO)[0].evolution[0]
        );
    }

    #[test]
    fn a_fire_grows_and_leans_without_a_jump() {
        // More sprites and taller ones as it grows, each coming in from nothing.
        let mut last = 0.0f32;
        let mut count = 0;
        for step in 3..=100 {
            let now = sprites(&fire(f64::from(step) / 100.0), 5.0, DVec3::ZERO);
            assert!(now.len() >= count);
            if now.len() > count && count > 0 {
                assert!(now.last().unwrap().extra[0] < 0.2);
            }
            count = now.len();
            assert!(now[0].sphere[3] >= last);
            last = now[0].sphere[3];
        }
        // Downwind, and never flat.
        let windy = sprites(&fire(0.9), 5.0, DVec3::new(40.0, 0.0, 0.0));
        for puff in &windy {
            assert!(puff.quad[0] > 0.5);
            assert!(puff.quad[0].hypot(puff.quad[2]) < (MAX_TILT.tan() + 0.08) as f32);
        }
        // A fire that is out draws nothing.
        assert!(sprites(&fire(0.0), 5.0, DVec3::ZERO).is_empty());
    }

    #[test]
    fn a_wide_fire_lies_over_in_a_fresh_wind_and_a_small_one_stands_in_light_air() {
        let tilt = |lean: DVec3| lean.length().atan().to_degrees();
        // A roof 15 m across stands upright in 1.5 m/s and leans 39° in 3 m/s, downwind.
        assert_eq!(lean_in(DVec3::new(1.5, 0.0, 0.0), 15.0), DVec3::ZERO);
        let roof = lean_in(DVec3::new(3.0, 0.0, 0.0), 15.0);
        assert!(
            (tilt(roof) - 38.7).abs() < 0.5 && roof.x > 0.0,
            "{:.1}°",
            tilt(roof)
        );
        // A hatch 3 m across stands upright in 1 m/s and leans 43° in 2 m/s.
        assert_eq!(lean_in(DVec3::new(1.0, 0.0, 0.0), 3.0), DVec3::ZERO);
        let small = tilt(lean_in(DVec3::new(0.0, 0.0, -2.0), 3.0));
        assert!((small - 43.0).abs() < 0.5, "{small:.1}°");
        // In 9 m/s the roof would lie at 63°: the card stops at 50°, as in a gale.
        for speed in [9.0, 30.0] {
            let gale = tilt(lean_in(DVec3::new(speed, 0.0, 0.0), 15.0));
            assert!((gale - 50.0).abs() < 0.01, "{gale:.2}°");
        }
    }

    #[test]
    fn a_fire_under_way_leans_as_the_air_past_it_does() {
        // Her way leans it aft, and a following wind faster than her takes that back.
        let mut small = fire(0.5);
        small.velocity = DVec3::new(6.0, 0.0, 0.0);
        assert!(
            sprites(&small, 5.0, DVec3::ZERO)
                .iter()
                .all(|p| p.quad[0] < -0.3)
        );
        let following = sprites(&small, 5.0, DVec3::new(10.5, 0.0, 0.0));
        assert!(following.iter().all(|p| p.quad[0] > 0.3));
    }
}
