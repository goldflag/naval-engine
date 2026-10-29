//! USDA NAIP aerial imagery for the ground nearest the camera in the continental United States
//! (`native/docs/terrain/surface.md`, "Near tiles"). Presentation only, as the rest of the theatre's land.
//!
//! - **Source.** The U.S. Geological Survey's The National Map serves the National Agriculture Imagery Program's
//!   mosaic (USDA Farm Service Agency, 0.6 m, four bands, natural colour) as an ArcGIS ImageServer
//!   ([`EXPORT_URL`]): `exportImage` answers any web-mercator box, no key, no account, no charge. NAIP and The National
//!   Map's products are public domain; the credit asked for is [`ATTRIBUTION`]. (AWS's NAIP buckets are requester-pays,
//!   USDA's own APFO server did not answer from here: neither is used.)
//! - **Tiles.** Fixed squares of web mercator, each zoom-[`ZOOM`] tile asked for at [`TILE_PX`]² pixels (zoom 16's pixel:
//!   2.4 m at the equator, 1.9 m at 37°), so a tile asked once is a file of the surface's cache for every scenario
//!   (`surface-cache/naip-usgs/15/{x}/{y}.img`, JPEG, or PNG with transparency where the mosaic has no image: the sea,
//!   Mexico, Canada).
//! - **Where.** Only for near tiles (`theatre::near`, at [`TEXEL_M`] where the scenario stands in [`in_conus`]), so only
//!   within a few kilometres of the camera; Sentinel-2 stays the colour everywhere else and wherever NAIP has none.
//!   Its colour is matched to Sentinel-2's at a scale of [`MATCH_TEXELS`] texels (`surface::assemble`): NAIP's detail
//!   over Sentinel-2's broad colour, so the land's calibration (native/docs/terrain/surface.md, "The photograph as
//!   albedo") holds and no seam shows where NAIP ends.
//! - **Switched** with the surface's streaming; `NAVAL_NAIP=off` keeps Sentinel-2 alone.

use std::sync::OnceLock;

use naval_earth::geo::LatLon;

use super::surface;

/// The National Map's NAIP ImageServer.
pub const EXPORT_URL: &str =
    "https://imagery.nationalmap.gov/arcgis/rest/services/USGSNAIPImagery/ImageServer/exportImage";
/// What the game credits for the imagery.
pub const ATTRIBUTION: &str =
    "NAIP imagery: USDA Farm Service Agency, via the USGS National Map (public domain)";
/// The web-mercator zoom of a tile's square.
pub const ZOOM: u32 = 15;
/// Pixels on a tile's side (zoom 16's pixel over a zoom-15 square).
pub const TILE_PX: i64 = 512;
/// A near tile's texel where NAIP is the colour, metres (512 texels: a kilometre a tile, eight kilometres a sea window's
/// block about the camera).
pub const TEXEL_M: f64 = 2.0;
/// NAIP's colour is matched to Sentinel-2's over squares of this many texels (64 m at 2 m: about four of its pixels).
pub const MATCH_TEXELS: usize = 32;

/// Whether NAIP is fetched: with the surface's streaming, unless `NAVAL_NAIP` is `off` (or `0`).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_NAIP").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off
}

/// The conterminous United States and its near waters, generously (lon, lat): the server's own transparency marks
/// where its mosaic ends.
const CONUS: [[f64; 2]; 40] = [
    [-124.9, 49.0],
    [-95.2, 49.0],
    [-89.5, 48.1],
    [-84.5, 46.6],
    [-82.4, 45.2],
    [-82.5, 42.0],
    [-79.0, 42.8],
    [-76.5, 44.0],
    [-74.7, 45.0],
    [-71.5, 45.0],
    [-69.2, 47.5],
    [-67.7, 47.1],
    [-66.8, 44.8],
    [-69.8, 41.2],
    [-73.9, 40.3],
    [-74.9, 38.5],
    [-75.3, 35.2],
    [-80.7, 31.9],
    [-79.9, 26.5],
    [-80.1, 24.4],
    [-82.0, 24.4],
    [-82.9, 27.5],
    [-84.3, 29.6],
    [-89.4, 28.8],
    [-94.0, 29.3],
    [-97.2, 25.8],
    [-99.5, 27.4],
    [-101.4, 29.7],
    [-104.5, 29.4],
    [-106.5, 31.7],
    [-108.2, 31.3],
    [-111.1, 31.3],
    [-114.8, 32.5],
    [-117.1, 32.5],
    [-117.6, 32.4],
    [-120.8, 34.3],
    [-122.7, 36.8],
    [-123.9, 38.4],
    [-124.7, 40.3],
    [-124.9, 43.0],
];

/// Whether a point is in the conterminous United States or its near waters (where NAIP has imagery).
pub fn in_conus(lat: f64, lon: f64) -> bool {
    // Even-odd over the outline.
    let mut inside = false;
    let n = CONUS.len();
    for k in 0..n {
        let [x0, y0] = CONUS[k];
        let [x1, y1] = CONUS[(k + 1) % n];
        if (y0 > lat) != (y1 > lat) && lon < x0 + (lat - y0) * (x1 - x0) / (y1 - y0) {
            inside = !inside;
        }
    }
    inside
}

/// Whether near tiles in the scenario about `geo` take NAIP (and its finer texel): NAIP on and the scenario's origin in
/// the conterminous United States. (Its tiles past the border or out at sea take Sentinel-2, at NAIP's texel.)
pub fn serves(geo: LatLon) -> bool {
    enabled() && in_conus(geo.lat, geo.lon)
}

/// Tile (x, y)'s request: its web-mercator box at [`TILE_PX`]² pixels.
pub fn tile_url(x: i64, y: i64) -> String {
    let side = 2.0 * std::f64::consts::PI * 6_378_137.0 / f64::from(1u32 << ZOOM);
    let half = std::f64::consts::PI * 6_378_137.0;
    let x0 = -half + x as f64 * side;
    let y1 = half - y as f64 * side;
    format!(
        "{EXPORT_URL}?bbox={:.3},{:.3},{:.3},{:.3}&bboxSR=3857&imageSR=3857&size={TILE_PX},{TILE_PX}&format=jpgpng&f=image",
        x0,
        y1 - side,
        x0 + side,
        y1
    )
}

/// A tile's bytes decoded: RGBA, `TILE_PX`² (alpha 0 where the mosaic has no image); none if not an image of that size.
pub fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    (rgba.width() == TILE_PX as u32 && rgba.height() == TILE_PX as u32).then(|| rgba.into_raw())
}

/// Whether bytes a server sent are an image (JPEG or PNG), not an error page.
pub fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff]) || bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conterminous_united_states_and_nowhere_else() {
        for (name, lat, lon) in [
            ("Santa Cruz", 36.97, -122.03),
            ("Monterey Bay", 36.8, -122.0),
            ("San Diego", 32.72, -117.16),
            ("Seattle", 47.6, -122.33),
            ("Denver", 39.74, -104.99),
            ("New York", 40.71, -74.0),
            ("Miami", 25.76, -80.19),
            ("Key West", 24.55, -81.78),
            ("Chicago", 41.88, -87.63),
            ("Bar Harbor", 44.39, -68.2),
            ("Galveston", 29.3, -94.8),
        ] {
            assert!(in_conus(lat, lon), "{name}");
        }
        for (name, lat, lon) in [
            ("Tijuana's south", 32.3, -116.9),
            ("Toronto", 43.65, -79.38),
            ("Vancouver", 49.28, -123.1),
            ("Montreal", 45.5, -73.6),
            ("the Gulf of Mexico", 26.0, -90.0),
            ("Honolulu", 21.3, -157.86),
            ("Havana", 23.1, -82.4),
            ("Reykjavik", 64.1, -21.9),
            ("Guadalcanal", -9.4, 160.0),
            ("the open Pacific", 36.9, -126.0),
        ] {
            assert!(!in_conus(lat, lon), "{name}");
        }
    }

    #[test]
    fn a_tiles_request_is_its_mercator_square() {
        // Santa Cruz's zoom-15 tile: the box's corners are the tile's, 512 pixels a side.
        let t = naval_osm::TileId::of(36.97, -122.03, 15);
        let url = tile_url(i64::from(t.x), i64::from(t.y));
        assert!(
            url.starts_with(EXPORT_URL)
                && url.contains("size=512,512")
                && url.contains("bboxSR=3857")
        );
        let bbox: Vec<f64> = url
            .split("bbox=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .split(',')
            .map(|v| v.parse().unwrap())
            .collect();
        let side = bbox[2] - bbox[0];
        assert!((side - 1222.99).abs() < 0.1, "{side}");
        assert!((bbox[3] - bbox[1] - side).abs() < 1e-6);
        // Its north-west corner, back to latitude and longitude, is the tile's.
        let ([_, west], [north, _]) = t.bounds();
        let lon = (bbox[0] / 6_378_137.0).to_degrees();
        assert!((lon - west).abs() < 1e-6, "{lon} {west}");
        let lat =
            (2.0 * (bbox[3] / 6_378_137.0).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
        assert!((lat - north).abs() < 1e-6, "{lat} {north}");
        assert!(is_image(&[0xff, 0xd8, 0xff, 0xe0]) && is_image(b"\x89PNG\r\n"));
        assert!(!is_image(b"{\"error\":"));
    }
}
