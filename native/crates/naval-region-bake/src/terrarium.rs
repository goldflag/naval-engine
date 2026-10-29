//! AWS Terrain Tiles (Terrarium): 256² web-mercator PNGs whose RGB encode metres as R·256 + G + B/256 − 32768. Their
//! sources (tilezen/joerd): SRTM and GMTED2010 (U.S. Geological Survey), ETOPO1 (NOAA), EU-DEM, ArcticDEM, and on the US
//! coast USGS NED.

use crate::BakeError;

pub const TERRARIUM_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium";
pub const TILE_PX: usize = 256;
/// A decoded tile: 256 × 256 heights, metres, row-major.
pub type DemTile = Vec<f32>;

/// USGS NED's fill over coastal sea, 4 ft as Terrarium encodes it: Terrarium carries it on the US coast in rectangular
/// blocks up to 7 km out to sea (Big Sur, Monterey Bay). Within half a quantum of it a sample is sea level, as
/// Terrarium gives other coastal water (bake 3, 2026-10-04).
pub const NED_SEA_FILL_M: f64 = 1.21875;

/// The zoom whose pixel is about `cell_m` at `lat`: z11 at 9° S (75 m), z10 at 64.6° N (67 m).
pub fn zoom_for(lat: f64, cell_m: f64) -> u32 {
    let pixel = 2.0 * std::f64::consts::PI * 6_378_137.0 / 256.0;
    (pixel * (lat * std::f64::consts::PI / 180.0).cos() / cell_m)
        .log2()
        .round()
        .clamp(1.0, 14.0) as u32
}

/// A tile's path under [`TERRARIUM_URL`], x wrapped onto the world.
pub fn tile_path(zoom: u32, x: i64, y: i64) -> String {
    let n = 1i64 << zoom;
    format!("{zoom}/{}/{y}.png", x.rem_euclid(n))
}

/// RGB to metres, NED's sea fill at sea level.
pub fn heights(rgb: &[u8]) -> DemTile {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let h = p[0] as f64 * 256.0 + p[1] as f64 + p[2] as f64 / 256.0 - 32768.0;
            if (h - NED_SEA_FILL_M).abs() <= 1.0 / 256.0 {
                0.0
            } else {
                h as f32
            }
        })
        .collect()
}

/// A Terrarium PNG's heights. No colour management touches the encoded values.
pub fn decode(png_bytes: &[u8]) -> Result<DemTile, BakeError> {
    rgb(png_bytes)
        .map(|rgb| heights(&rgb))
        .map_err(|e| BakeError::Corrupt(format!("a Terrarium tile does not decode: {e}")))
}

/// A 256² 8-bit PNG's pixels as RGB bytes, row-major, whatever its colour type (an elevation tile's encoded values:
/// Terrarium's, or the Geospatial Information Authority of Japan's).
pub fn rgb(png_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| "too large".to_string())?
    ];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    if info.width as usize != TILE_PX
        || info.height as usize != TILE_PX
        || info.bit_depth != png::BitDepth::Eight
    {
        return Err(format!(
            "{} × {} at {:?} bits, not 256² at 8",
            info.width, info.height, info.bit_depth
        ));
    }
    let pixels = &buffer[..info.buffer_size()];
    let rgb: Vec<u8> = match info.color_type {
        png::ColorType::Rgb => pixels.to_vec(),
        png::ColorType::Rgba => pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0]])
            .collect(),
        other => return Err(format!("colour type {other:?}")),
    };
    Ok(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neds_sea_fill_reads_as_sea_level_and_other_heights_as_encoded() {
        // The fill 1.21875 and the value a quantum below it, 1.2 m land, 1.5 m land, 10 m deep.
        let rgb = [128, 1, 56, 128, 1, 55, 128, 1, 51, 128, 1, 128, 127, 246, 0];
        assert_eq!(heights(&rgb), [0.0, 0.0, 1.199_218_8, 1.5, -10.0]);
    }

    #[test]
    fn the_zoom_follows_the_latitude() {
        assert_eq!(zoom_for(-9.3, 70.0), 11);
        assert_eq!(zoom_for(64.6, 70.0), 10);
        assert_eq!(zoom_for(35.6, 70.0), 11);
        assert_eq!(tile_path(11, -3, 900), "11/2045/900.png");
    }
}
