//! Oil burning on the water: the patches of a lost ship's slick that her own fire, going down with her in a calm sea,
//! set alight (naval-ocean's `oil::OilSlick::fires` and `oil::sea_holds_a_burn`, handed over by the host), drawn by the
//! ship fires' own presentation (`fires.rs`): flames, the black smoke column, the light. Each burning slick is a frame
//! of its own whose upperworks zones are cells of the sea about where it was lit, [`CELL`] m square; a cell burns where
//! a burning patch's core covers its middle and the oil lies thick in a pool there, as hard as its film allows (a few
//! millimetres and more, all of it).
//!
//! What it looks like follows the pool fires measured on the sea: heavy fuel oil burns low and smoky, a fire tens of
//! metres across standing a fraction of its width high (Heskestad's flame height for a 16 m cell at the half megawatt
//! a square metre heavy oil gives, 0.235 Q^⅖ − 1.02 D, about 7 m, as `fires.rs` draws a zone's flames: 1.5 + 3.6 × its
//! intensity, times its scale), its smoke black (soot yields of 8 to 15 %: Fingas 2007; Evans et al., J. Res. NIST
//! 106, 2001). A cell's heat is the zone's, 63 MW at full intensity (`FIRE_HEAT_KW` × its scale squared: a quarter of
//! a megawatt a square metre, within the measured range).

use std::collections::HashMap;
use std::sync::Arc;

use glam::DVec3;

use crate::math::ShipPose;
use crate::particles::SourceId;
use crate::ships::{FireState, ShipEffectsProfile, ShipFrame, ZoneVent};

/// A cell's edge (m).
pub const CELL: f64 = 16.0;
/// Cells either way of where it was lit: some 400 m across in all.
const REACH: i32 = 12;
const SIDE: usize = (2 * REACH + 1) as usize;
/// The film (m) at which a cell burns at its full intensity, and the least intensity a burning cell shows.
const FULL_FILM: f64 = 0.005;
const LEAST: f64 = 0.3;
/// Seconds a slick's frame is kept after its last patch went out, so its fires die down as fires do.
const KEPT_SECONDS: f64 = 60.0;
/// The oil on the sea lies in thick pools with thin film and open water between (the bulk of a slick's oil in a small
/// share of its area: ITOPF, *Aerial observation of marine oil spills*, TIP 1), and a fire on it burns only where it
/// lies thick: a cell's film is its pool's share of the slick's (`pool`), none between the pools and up to
/// [`POOL_THICKEST`] times it in a pool's middle. Each [`POOL_SPACING`]-cell square of the grid holds a pool with
/// chance [`POOL_CHANCE`], its middle anywhere in the square's inner half, [`POOL_RADII`] cells in radius: two thirds
/// to three quarters of the sea open, the pools some 25 to 55 m across and mostly apart (their columns rise apart and
/// merge aloft, `fires.rs`), each slick's laid out its own way. They lie on the oil (the grid drifts with it).
const POOL_SPACING: f64 = 3.5;
const POOL_CHANCE: f64 = 0.8;
const POOL_RADII: [f64; 2] = [0.8, 1.7];
const POOL_THICKEST: f64 = 2.2;

/// A burning patch of a slick, as naval-ocean hands it over (`oil::OilFire`): where (world x, z), across which way, its
/// core's half-breadth and half-length (m) and its film (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OilFire {
    pub x: f64,
    pub z: f64,
    pub right: [f64; 2],
    pub width: f64,
    pub length: f64,
    pub film: f64,
}

#[derive(Debug)]
struct Burning {
    /// Where it was lit (world x, z): the cells' grid is laid about it.
    anchor: [f64; 2],
    /// How its pools lie, from its id.
    seed: u64,
    /// The effects' clock when a patch last burned.
    last: f64,
    frame: ShipFrame,
}

/// The burning slicks, as frames for the fires.
#[derive(Debug, Default)]
pub struct SeaFires {
    slicks: HashMap<SourceId, Burning>,
    frames: Vec<ShipFrame>,
    profile: Option<Arc<ShipEffectsProfile>>,
    time: f64,
}

/// Three draws in [0, 1) from a cell's number, the same on every run.
fn draws(k: usize) -> [f64; 3] {
    let mut h = (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5851_f42d_4c95_7f2d;
    std::array::from_fn(|_| {
        h ^= h >> 31;
        h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h ^= h >> 29;
        (h >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// The cells' frame: every cell a zone on the sea about the grid's middle, its middle anywhere in the inner two thirds
/// of its square and its size two thirds to four thirds of a square's (a pool fire's flames break up into patches:
/// laid on a grid, they would stand in rows).
fn profile() -> Arc<ShipEffectsProfile> {
    let zones = (0..SIDE * SIDE)
        .map(|k| {
            let (i, j) = ((k % SIDE) as i32 - REACH, (k / SIDE) as i32 - REACH);
            let [a, b, c] = draws(k);
            let size = CELL * (0.67 + 0.67 * c);
            Some(ZoneVent {
                position: DVec3::new(
                    (f64::from(i) + (a - 0.5) * 0.67) * CELL,
                    0.2,
                    (f64::from(j) + (b - 0.5) * 0.67) * CELL,
                ),
                half: [size * 0.42, size * 0.42],
                scale: size / 11.0,
            })
        })
        .collect();
    Arc::new(ShipEffectsProfile {
        preset_id: "burning oil".into(),
        on_the_water: true,
        hull_length: 2.0 * f64::from(REACH) * CELL,
        hull_beam: 2.0 * f64::from(REACH) * CELL,
        zones,
        ..Default::default()
    })
}

impl SeaFires {
    /// The burning slicks' frames, for the fires.
    pub fn frames(&self) -> &[ShipFrame] {
        &self.frames
    }

    /// Cells burning now, for tools and tests.
    pub fn cells(&self) -> usize {
        self.frames
            .iter()
            .map(|f| f.zone_fires.iter().filter(|z| z.intensity > 0.0).count())
            .sum()
    }

    pub fn reset(&mut self) {
        self.slicks.clear();
        self.frames.clear();
    }

    /// Moves what burns by `offset` (a theatre scene's re-centre).
    pub fn translate(&mut self, offset: DVec3) {
        for burning in self.slicks.values_mut() {
            burning.anchor = [burning.anchor[0] + offset.x, burning.anchor[1] + offset.z];
        }
    }

    /// This frame's burning slicks, each by the id of the hull whose oil it is, after `dt` seconds in the `wind` (m/s,
    /// world): their cells lit where their patches burn, the grid carried with the surface as the oil is
    /// (`flotsam::DRIFT_SHARE` of the wind, naval-ocean's `oil::DRIFT_SHARE`), so a cell keeps to the oil it lies on. A
    /// slick no longer burning is kept a minute with its cells out.
    pub fn set<'a>(
        &mut self,
        dt: f64,
        wind: DVec3,
        slicks: impl IntoIterator<Item = (&'a str, &'a [OilFire])>,
    ) {
        let dt = dt.max(0.0);
        self.time += dt;
        let drift = [wind.x, wind.z].map(|w| w * crate::flotsam::DRIFT_SHARE * dt);
        if drift.iter().all(|d| d.is_finite()) {
            for burning in self.slicks.values_mut() {
                burning.anchor = [burning.anchor[0] + drift[0], burning.anchor[1] + drift[1]];
            }
        }
        let now = self.time;
        let profile = self.profile.get_or_insert_with(profile).clone();
        for burning in self.slicks.values_mut() {
            burning
                .frame
                .zone_fires
                .iter_mut()
                .for_each(|z| z.intensity = 0.0);
        }
        for (ship, fires) in slicks {
            if fires.is_empty() {
                continue;
            }
            let id: SourceId = Arc::from(format!("oil:{ship}"));
            let burning = self.slicks.entry(id.clone()).or_insert_with(|| {
                // Laid about the middle of what burns when it is first seen.
                let n = fires.len() as f64;
                let anchor = [
                    fires.iter().map(|f| f.x).sum::<f64>() / n,
                    fires.iter().map(|f| f.z).sum::<f64>() / n,
                ];
                let mut frame = ShipFrame::new(
                    &id,
                    profile.clone(),
                    ShipPose {
                        x: anchor[0],
                        z: anchor[1],
                        ..ShipPose::default()
                    },
                );
                frame.zone_fires = vec![FireState::default(); SIDE * SIDE];
                Burning {
                    anchor,
                    seed: seed_of(&id),
                    last: now,
                    frame,
                }
            });
            let seed = burning.seed;
            burning.last = now;
            burning.frame.pose.x = burning.anchor[0];
            burning.frame.pose.z = burning.anchor[1];
            for fire in fires {
                // The cells whose middles its core covers.
                let reach = fire.width.max(fire.length);
                let cells = |at: f64, from: f64| {
                    // A square further either way: its cell's middle may stand a third of a square into this one.
                    let low = ((at - reach - from) / CELL).floor() as i32 - 1;
                    let high = ((at + reach - from) / CELL).ceil() as i32 + 1;
                    low.max(-REACH)..=high.min(REACH)
                };
                for j in cells(fire.z, burning.anchor[1]) {
                    for i in cells(fire.x, burning.anchor[0]) {
                        let k = (j + REACH) as usize * SIDE + (i + REACH) as usize;
                        let Some(Some(zone)) = profile.zones.get(k) else {
                            continue;
                        };
                        let (cx, cz) = (
                            burning.anchor[0] + zone.position.x,
                            burning.anchor[1] + zone.position.z,
                        );
                        let (dx, dz) = (cx - fire.x, cz - fire.z);
                        let across =
                            (dx * fire.right[0] + dz * fire.right[1]) / fire.width.max(1.0);
                        let along =
                            (-dx * fire.right[1] + dz * fire.right[0]) / fire.length.max(1.0);
                        if across * across + along * along > 1.0 {
                            continue;
                        }
                        // Thick in its pools, thin or open between them: it burns where it lies thick.
                        let film = fire.film * pool(seed, i, j);
                        if film <= RESIDUE {
                            continue;
                        }
                        let intensity =
                            ((film - RESIDUE) / (FULL_FILM - RESIDUE)).clamp(LEAST, 1.0);
                        let cell = &mut burning.frame.zone_fires[k];
                        cell.intensity = cell.intensity.max(intensity);
                    }
                }
            }
        }
        self.slicks.retain(|_, b| now - b.last < KEPT_SECONDS);
        self.frames.clear();
        self.frames
            .extend(self.slicks.values().map(|b| b.frame.clone()));
        // Their order is the map's: kept stable for the fires' ranking.
        self.frames.sort_by(|a, b| a.id.cmp(&b.id));
    }
}

/// The film (m) a burning patch goes out at (naval-ocean's `oil::RESIDUE_M`).
const RESIDUE: f64 = 0.001;

/// A slick's pools' seed, from its id (FNV-1a): the same slick lies the same way on every run.
fn seed_of(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The share of a slick's film that lies at cell (`i`, `j`) of its grid: thickest in a pool's middle
/// ([`POOL_THICKEST`]), thinning to its edge, none between the pools.
fn pool(seed: u64, i: i32, j: i32) -> f64 {
    let (x, z) = (f64::from(i) / POOL_SPACING, f64::from(j) / POOL_SPACING);
    let (x0, z0) = (x.floor() as i64, z.floor() as i64);
    let mut most = 0.0_f64;
    // A pool reaches at most half a square past its own.
    for b in z0 - 1..=z0 + 1 {
        for a in x0 - 1..=x0 + 1 {
            let mut h = seed
                ^ (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
            let [chance, u, v, r]: [f64; 4] = std::array::from_fn(|_| {
                h ^= h >> 31;
                h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
                h ^= h >> 29;
                (h >> 11) as f64 / (1u64 << 53) as f64
            });
            if chance >= POOL_CHANCE {
                continue;
            }
            let radius = (POOL_RADII[0] + (POOL_RADII[1] - POOL_RADII[0]) * r) / POOL_SPACING;
            let (cx, cz) = (a as f64 + 0.25 + 0.5 * u, b as f64 + 0.25 + 0.5 * v);
            let d2 = ((x - cx).powi(2) + (z - cz).powi(2)) / (radius * radius);
            most = most.max(1.0 - d2);
        }
    }
    POOL_THICKEST * most
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fire(x: f64, z: f64, width: f64, film: f64) -> OilFire {
        OilFire {
            x,
            z,
            right: [1.0, 0.0],
            width,
            length: width,
            film,
        }
    }

    #[test]
    fn a_burning_slick_lights_the_cells_its_patches_cover() {
        let mut sea = SeaFires::default();
        // A burning slick 80 m about, 1 cm thick.
        let fires = [fire(100.0, 50.0, 80.0, 0.01)];
        sea.set(0.1, DVec3::ZERO, [("bismarck", &fires[..])]);
        assert_eq!(sea.frames().len(), 1);
        let frame = &sea.frames()[0];
        assert_eq!(&*frame.id, "oil:bismarck");
        // Its core covers some π 80² / 16² ≈ 78 cells' middles; it burns only in its pools, with open water between,
        // harder where it lies thicker.
        let lit: Vec<f64> = frame
            .zone_fires
            .iter()
            .map(|z| z.intensity)
            .filter(|&i| i > 0.0)
            .collect();
        assert!((10..=50).contains(&lit.len()), "{} cells", lit.len());
        assert!(lit.contains(&1.0) && lit.iter().any(|&i| i < 0.8));
        // Each lit cell's middle is on the sea, where a core covers it.
        for (k, cell) in frame.zone_fires.iter().enumerate() {
            if cell.intensity <= 0.0 {
                continue;
            }
            let zone = frame.profile.zones[k].as_ref().unwrap();
            let at = frame.pose.local_to_world(zone.position);
            assert!(at.y > 0.0 && at.y < 1.0);
            assert!(
                fires
                    .iter()
                    .any(|f| (at.x - f.x).hypot(at.z - f.z) <= f.width + 1e-6)
            );
        }
        // Out: its cells go out, and it is forgotten a minute on.
        sea.set(1.0, DVec3::ZERO, std::iter::empty());
        assert_eq!(sea.cells(), 0);
        assert_eq!(sea.frames().len(), 1);
        sea.set(KEPT_SECONDS, DVec3::ZERO, std::iter::empty());
        assert!(sea.frames().is_empty());
    }

    /// The oil lies in pools with open water between: half to four fifths of the grid open, the rest from thin to
    /// twice the slick's film, in many pools apart rather than one field, and each slick's laid out its own way.
    #[test]
    fn the_oil_lies_in_pools_apart() {
        let grid = |seed: u64| -> Vec<f64> {
            (-REACH..=REACH)
                .flat_map(|j| (-REACH..=REACH).map(move |i| pool(seed, i, j)))
                .collect()
        };
        for ship in ["bismarck", "hood", "king-george-v"] {
            let cells = grid(seed_of(&format!("oil:{ship}")));
            let lit = cells.iter().filter(|&&p| p > 0.0).count();
            let open = 1.0 - lit as f64 / cells.len() as f64;
            let thickest = cells.iter().copied().fold(0.0, f64::max);
            // The pools: the lit cells' groups, neighbour by neighbour.
            let mut seen = vec![false; cells.len()];
            let mut pools = Vec::new();
            for k in 0..cells.len() {
                if seen[k] || cells[k] <= 0.0 {
                    continue;
                }
                let (mut stack, mut size) = (vec![k], 0);
                seen[k] = true;
                while let Some(c) = stack.pop() {
                    size += 1;
                    let (i, j) = (c % SIDE, c / SIDE);
                    let next = [
                        (i > 0).then(|| c - 1),
                        (i + 1 < SIDE).then(|| c + 1),
                        (j > 0).then(|| c - SIDE),
                        (j + 1 < SIDE).then(|| c + SIDE),
                    ];
                    for n in next.into_iter().flatten() {
                        if !seen[n] && cells[n] > 0.0 {
                            seen[n] = true;
                            stack.push(n);
                        }
                    }
                }
                pools.push(size);
            }
            let largest = pools.iter().copied().max().unwrap_or(0);
            println!(
                "{ship}: open {open:.2}, thickest {thickest:.2}, {} pools, largest {largest}",
                pools.len()
            );
            assert!((0.5..0.8).contains(&open), "{open:.2} of the grid open");
            assert!(thickest > 1.5);
            assert!(pools.len() >= 15, "{} pools", pools.len());
            assert!(
                largest * 3 < lit,
                "the largest pool {largest} of {lit} cells"
            );
        }
        let (a, b) = (grid(seed_of("oil:hood")), grid(seed_of("oil:bismarck")));
        let unlike = a
            .iter()
            .zip(&b)
            .filter(|(a, b)| (**a > 0.0) != (**b > 0.0))
            .count();
        assert!(unlike * 5 > a.len(), "{unlike} cells differ");
    }
}
