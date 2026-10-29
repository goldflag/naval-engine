//! Presentation-only geographic water. Broad offshore profiles and a small authored atlas of coastal areas.
//! These are representative waters, not historical ocean-colour observations. Latitude supplies a conservative
//! fallback for arbitrary editor regions; it is not a claim that temperature determines water colour. Authored
//! coastal areas override it independently of seabed depth and weather.

use naval_earth::geo::{LatLon, to_local, wrap_lon};
use naval_ocean::optics::{WaterField, WaterOptics};

struct Area {
    at: LatLon,
    radii: [f64; 2],
    optics: WaterOptics,
    strength: f32,
}

// Approximate water masses, ordered broad waters before smaller bays/plumes. The soft outer boundary is the given
// radius, the inner 39% is uniform. Distances are metres; the centres are geographic, never camera-relative.
const AREAS: &[Area] = &[
    Area {
        at: LatLon {
            lat: 56.0,
            lon: 3.0,
        },
        radii: [650_000.0, 850_000.0],
        optics: WaterOptics::PRODUCTIVE,
        strength: 0.8,
    },
    Area {
        at: LatLon {
            lat: 57.0,
            lon: 20.0,
        },
        radii: [450_000.0, 550_000.0],
        optics: WaterOptics::ORGANIC,
        strength: 0.7,
    },
    Area {
        at: LatLon {
            lat: 64.15,
            lon: -22.2,
        },
        radii: [60_000.0, 22_000.0],
        optics: WaterOptics::PRODUCTIVE,
        strength: 0.65,
    },
    Area {
        at: LatLon {
            lat: -9.25,
            lon: 160.05,
        },
        radii: [45_000.0, 18_000.0],
        optics: WaterOptics::PRODUCTIVE,
        strength: 0.65,
    },
    Area {
        at: LatLon {
            lat: -9.42,
            lon: 160.13,
        },
        radii: [6_000.0, 2_000.0],
        optics: WaterOptics::SEDIMENT,
        strength: 0.65,
    },
    Area {
        at: LatLon {
            lat: 50.1,
            lon: -1.0,
        },
        radii: [100_000.0, 40_000.0],
        optics: WaterOptics::SEDIMENT,
        strength: 0.55,
    },
    Area {
        at: LatLon {
            lat: -0.5,
            lon: -49.5,
        },
        radii: [160_000.0, 160_000.0],
        optics: WaterOptics::SEDIMENT,
        strength: 0.85,
    },
    Area {
        at: LatLon {
            lat: 29.0,
            lon: -89.4,
        },
        radii: [110_000.0, 70_000.0],
        optics: WaterOptics::SEDIMENT,
        strength: 0.8,
    },
    Area {
        at: LatLon {
            lat: 47.6,
            lon: -122.45,
        },
        radii: [20_000.0, 80_000.0],
        optics: WaterOptics::ORGANIC,
        strength: 0.55,
    },
];

fn offshore(origin: LatLon) -> WaterOptics {
    // Smooth fallback for regions without an authored offshore profile; clear low-nutrient water at low latitudes.
    let t = ((origin.lat.abs() - 25.0) / 20.0).clamp(0.0, 1.0) as f32;
    let base = WaterOptics::CLEAR.mix(WaterOptics::OCEANIC, t * t * (3.0 - 2.0 * t));
    let lon = wrap_lon(origin.lon);
    // Clear Mediterranean offshore waters extend beyond the low-latitude fallback.
    if (30.0..=46.0).contains(&origin.lat) && (-6.0..=36.0).contains(&lon) {
        WaterOptics::CLEAR.mix(base, 0.25)
    } else {
        base
    }
}

/// `render_origin` is a point in the scenario's persistent metre frame. Only centres are rebased; coefficients,
/// area sizes and overlap order remain identical when the camera follows another ship or crosses a scene boundary.
pub fn field(origin: LatLon, render_origin: [f64; 2]) -> WaterField {
    let mut water = WaterField::new(offshore(origin));
    for area in AREAS {
        let centre = to_local(origin, area.at);
        // A scenario spans at most 2,000 km. Cull distant atlas entries in scenario coordinates, independently of
        // the moving render origin, so GPU work stays small and no area appears/disappears during recentering.
        if (0..2).any(|i| centre[i].abs() > area.radii[i] + 2_000_000.0) {
            continue;
        }
        water
            .add_area(
                [centre[0] - render_origin[0], centre[1] - render_origin[1]],
                area.radii,
                area.optics,
                area.strength,
            )
            .expect("authored geographic water area");
    }
    water
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_stays_at_the_same_location_when_theatre_recentres() {
        let origin = LatLon {
            lat: -8.0,
            lon: 161.5,
        };
        let point = to_local(
            origin,
            LatLon {
                lat: -9.26,
                lon: 160.17,
            },
        );
        let first = field(origin, [0.0; 2]).sample(point.map(|v| v as f32), [1.0; 3]);
        let offset = [point[0] + 16_000.0, point[1] - 32_000.0];
        let second = field(origin, offset).sample(
            [(point[0] - offset[0]) as f32, (point[1] - offset[1]) as f32],
            [1.0; 3],
        );
        for (a, b) in first
            .absorption
            .into_iter()
            .chain(first.backscatter)
            .zip(second.absorption.into_iter().chain(second.backscatter))
        {
            assert!(
                (a - b).abs() < 1e-6,
                "water moved with the camera: {a} vs {b}"
            );
        }
    }

    #[test]
    fn both_builtin_theatres_and_custom_river_regions_have_geographic_water() {
        let tropics = LatLon {
            lat: -8.0,
            lon: 161.5,
        };
        let atlantic = LatLon {
            lat: 64.6,
            lon: -28.4,
        };
        let clear = field(tropics, [0.0; 2]).sample([0.0; 2], [1.0; 3]);
        let northern = field(atlantic, [0.0; 2]).sample([0.0; 2], [1.0; 3]);
        assert!(clear.beam()[2] < northern.beam()[2]);
        let bay = to_local(tropics, AREAS[3].at).map(|v| v as f32);
        assert!(field(tropics, [0.0; 2]).sample(bay, [1.0; 3]).absorption[2] > clear.absorption[2]);
        let river = LatLon {
            lat: 29.0,
            lon: -89.4,
        };
        assert!(
            field(river, [0.0; 2])
                .sample([0.0; 2], [1.0; 3])
                .backscatter[0]
                > 0.05
        );
    }

    #[test]
    fn equivalent_longitudes_select_the_same_water() {
        let a = field(
            LatLon {
                lat: -8.0,
                lon: 161.5,
            },
            [0.0; 2],
        );
        let b = field(
            LatLon {
                lat: -8.0,
                lon: -198.5,
            },
            [0.0; 2],
        );
        for p in [[0.0, 0.0], [-150_000.0, 140_000.0]] {
            let a = a.sample(p, [1.0; 3]);
            let b = b.sample(p, [1.0; 3]);
            for (x, y) in a.absorption.into_iter().zip(b.absorption) {
                assert!((x - y).abs() < 1e-6);
            }
        }
    }
}
