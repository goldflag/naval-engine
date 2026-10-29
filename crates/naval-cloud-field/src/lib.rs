//! The clouds' weather the simulation and the native clouds share, without either: takram's four cloud layers
//! ([`layers`]), the layers each weather draws ([`weathers`]), the local weather texture ([`LOCAL_WEATHER_PNG`], decoded
//! by [`weather_bytes`]) and how the cloud anchors' textures blend where more than one holds a place ([`blend`]). The
//! simulation's cloud field (`naval_sim::clouds`) re-exports them at their old paths; the native clouds read them here,
//! so they build without the simulation and the blend's tables stay one set of numbers on the CPU and the GPU.
//! The layer port and texture are MIT licensed; see THIRD_PARTY.md.
pub mod blend;
pub mod layers;
pub mod weathers;
/// The genera live in `naval-sky-types`, which the native sky reads without the simulation.
pub use naval_sky_types::{CloudGenus, HighCloudGenus, HighClouds};
pub use weathers::CloudWeather;

use std::{io::Cursor, sync::OnceLock};

pub const LOCAL_WEATHER_PNG: &[u8] = include_bytes!("local_weather.png");
pub const WEATHER_TILE_M: f64 = 100_000.0;

/// Decode once, with the same row flip as the native texture upload (the simulation samples it with the shader's
/// bilinear repeat, `naval_sim::clouds`' `weather_texel`).
pub fn weather_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let mut reader = png::Decoder::new(Cursor::new(LOCAL_WEATHER_PNG))
            .read_info()
            .expect("embedded weather PNG");
        let mut bytes = vec![0; reader.output_buffer_size().expect("weather size")];
        let info = reader.next_frame(&mut bytes).expect("weather texels");
        assert_eq!(
            (info.width, info.height, info.color_type),
            (512, 512, png::ColorType::Rgba)
        );
        let row = 512 * 4;
        (0..512)
            .rev()
            .flat_map(|y| bytes[y * row..(y + 1) * row].iter().copied())
            .collect()
    })
}
