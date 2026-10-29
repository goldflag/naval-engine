//! Fixtures for the release's date-line proofs (this crate's tests and the client's): a DEM that is a function of the
//! place, periodic round the world ([`dem`]), served as Terrarium tiles and GLO-30 cells named wrapped
//! ([`terrarium_tile`], [`terrarium_png`], [`glo30_cog`]), and the **reference** a square's surveyed heights must
//! match ([`reference`]): every supersampled point's bilinear height read straight from the function at the four pixels
//! about it, the pixels placed in unwrapped longitude, with no tiles, cells, sides or mosaics. A tile or cell placed a
//! turn wrong, or a mosaic built across a wrapped span, moves heights by tens of metres.
use std::io::Write;

use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};

use super::dem_at;
use super::sources::Reads;
use crate::geo::{mercator_lat, mercator_x, mercator_y};
use crate::square::bake_heights;
use crate::{Dem, glo30, terrarium};

/// Metres at (lat, lon): smooth and steep (tens of metres a kilometre), periodic in longitude.
pub fn dem(lat: f64, lon: f64) -> f64 {
    let (a, b) = (lat.to_radians(), lon.to_radians());
    120.0 * (b * 900.0).sin() + 80.0 * (a * 700.0).cos() + 30.0 * (a * 300.0 + b * 500.0).sin()
        - 40.0
}

/// Terrarium's RGB for a height, and back: the 1/256 m it keeps.
fn rgb(h: f64) -> [u8; 3] {
    let v = ((h + 32768.0) * 256.0).round().clamp(0.0, 16_777_215.0) as u32;
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The height of Terrarium pixel (`px`, `py`) at `scale` pixels round the world, `px` unwrapped, as a tile keeps it.
pub fn terrarium_pixel(px: i64, py: i64, scale: f64) -> f32 {
    let lat = mercator_lat(py as f64 + 0.5, scale);
    let lon = (px as f64 + 0.5) / scale * 360.0 - 180.0;
    terrarium::heights(&rgb(dem(lat, lon)))[0]
}

fn tile_rgb(z: u32, x: i64, y: i64) -> Vec<u8> {
    let scale = (1u64 << z) as f64 * 256.0;
    let mut out = Vec::with_capacity(256 * 256 * 3);
    for r in 0..256 {
        let lat = mercator_lat((y * 256 + r) as f64 + 0.5, scale);
        for c in 0..256 {
            let lon = ((x * 256 + c) as f64 + 0.5) / scale * 360.0 - 180.0;
            out.extend_from_slice(&rgb(dem(lat, lon)));
        }
    }
    out
}

/// Terrarium tile (z, x, y), x wrapped, decoded.
pub fn terrarium_tile(z: u32, x: i64, y: i64) -> Vec<f32> {
    terrarium::heights(&tile_rgb(z, x, y))
}

/// Terrarium tile (z, x, y), x wrapped, as the bucket serves it.
pub fn terrarium_png(z: u32, x: i64, y: i64) -> Vec<u8> {
    let mut out = vec![];
    let mut encoder = png::Encoder::new(&mut out, 256, 256);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("a PNG header");
    writer
        .write_image_data(&tile_rgb(z, x, y))
        .expect("a PNG body");
    writer.finish().expect("a PNG");
    out
}

/// GLO-30 cell (s, w)'s pixel (row, column), `w` wrapped or not.
pub fn glo30_pixel(s: i32, w: i32, row: usize, column: usize) -> f32 {
    let width = glo30::columns_at(s);
    dem(
        f64::from(s) + 1.0 - row as f64 / 3600.0,
        f64::from(w) + column as f64 / width as f64,
    ) as f32
}

/// GLO-30 cell (s, w), w wrapped, as the bucket serves it: a little-endian tiled TIFF of Float32 (DEFLATE, the
/// floating-point predictor, blocks of 1,024).
pub fn glo30_cog(s: i32, w: i32) -> Vec<u8> {
    let (width, height, block) = (glo30::columns_at(s), 3600usize, 1024usize);
    let (across, down) = (width.div_ceil(block), height.div_ceil(block));
    let mut blocks = vec![];
    for by in 0..down {
        for bx in 0..across {
            let stride = 4 * block;
            let mut raw = vec![0u8; block * stride];
            for r in 0..block {
                let mut row = vec![0u8; stride];
                for c in 0..block {
                    let (rr, cc) = (by * block + r, bx * block + c);
                    let v = if rr < height && cc < width {
                        glo30_pixel(s, w, rr, cc)
                    } else {
                        0.0
                    };
                    let b = v.to_be_bytes();
                    for (p, byte) in b.iter().enumerate() {
                        row[p * block + c] = *byte;
                    }
                }
                let mut previous = 0u8;
                for k in 0..stride {
                    raw[r * stride + k] = row[k].wrapping_sub(previous);
                    previous = row[k];
                }
            }
            let mut z = ZlibEncoder::new(vec![], Compression::fast());
            z.write_all(&raw).expect("deflate");
            blocks.push(z.finish().expect("deflate"));
        }
    }
    // Header, IFD, its arrays, then the blocks.
    let n = blocks.len();
    let entries: Vec<(u16, u16, Vec<u8>)> = {
        let short = |v: u16| v.to_le_bytes().to_vec();
        let long = |v: u32| v.to_le_bytes().to_vec();
        vec![
            (256, 4, long(width as u32)),
            (257, 4, long(height as u32)),
            (258, 3, short(32)),
            (259, 3, short(8)),
            (284, 3, short(1)),
            (317, 3, short(3)),
            (322, 3, short(block as u16)),
            (323, 3, short(block as u16)),
            (324, 4, vec![0; 4 * n]),
            (
                325,
                4,
                blocks
                    .iter()
                    .flat_map(|b| (b.len() as u32).to_le_bytes())
                    .collect(),
            ),
            (339, 3, short(3)),
            (
                33550,
                12,
                [1.0 / width as f64, 1.0 / 3600.0, 0.0]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
            (
                33922,
                12,
                [0.0, 0.0, 0.0, f64::from(w), f64::from(s) + 1.0, 0.0]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
        ]
    };
    let ifd = 8usize;
    let mut extra = ifd + 2 + 12 * entries.len() + 4;
    let mut out = vec![0x49, 0x49, 0x2a, 0x00];
    out.extend_from_slice(&(ifd as u32).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut arrays = vec![];
    let mut offsets_at = 0;
    let data_start = extra
        + entries
            .iter()
            .filter(|e| e.2.len() > 4)
            .map(|e| e.2.len())
            .sum::<usize>();
    for (tag, kind, value) in &entries {
        let size = match kind {
            3 => 2,
            4 => 4,
            _ => 8,
        };
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&((value.len() / size) as u32).to_le_bytes());
        if value.len() <= 4 {
            let mut v = value.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            if *tag == 324 {
                offsets_at = extra;
            }
            out.extend_from_slice(&(extra as u32).to_le_bytes());
            arrays.extend_from_slice(value);
            extra += value.len();
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&arrays);
    assert_eq!(out.len(), data_start);
    let mut at = data_start;
    for (k, b) in blocks.iter().enumerate() {
        out[offsets_at + 4 * k..offsets_at + 4 * k + 4].copy_from_slice(&(at as u32).to_le_bytes());
        at += b.len();
    }
    for b in blocks {
        out.extend_from_slice(&b);
    }
    out
}

/// The reference for square `reads.key`'s surveyed heights over [`dem`] served as above: each supersampled point's
/// bilinear height from the four pixels about it, each pixel's height computed at its place in unwrapped longitude;
/// then the same despike. Before the 1/256 m quantum a square keeps.
pub fn reference(reads: &Reads) -> Vec<f32> {
    // (column, row) of a point in its DEM's pixels, and a pixel's height.
    type Pixel = Box<dyn Fn(f64, f64) -> f64>;
    let place = |lon: f64, lat: f64| -> (f64, f64, Pixel) {
        if dem_at(lat) == Dem::Terrarium {
            let t = reads.terrarium.as_ref().expect("Terrarium reads");
            let scale = (1u64 << t.zoom) as f64 * 256.0;
            (
                mercator_x(lon, scale) - 0.5,
                mercator_y(lat, scale) - 0.5,
                Box::new(move |px, py| f64::from(terrarium_pixel(px as i64, py as i64, scale))),
            )
        } else {
            let part = reads
                .glo30
                .iter()
                .find(|p| p.serves(lat))
                .expect("GLO-30 reads");
            let win = part.window;
            let width = glo30::columns_at(win.s0);
            assert!(
                (win.s0..=win.s1).all(|s| glo30::columns_at(s) == width),
                "the reference reads one GLO-30 band"
            );
            (
                (lon - win.w0 as f64) / win.dx,
                (win.s1 as f64 + 1.0 - lat) / win.dy,
                Box::new(move |c, r| {
                    f64::from(
                        dem(win.s1 as f64 + 1.0 - r * win.dy, win.w0 as f64 + c * win.dx) as f32,
                    )
                }),
            )
        }
    };
    let value = |lon: f64, lat: f64| -> f64 {
        let (fx, fy, at) = place(lon, lat);
        let (ix, iy) = (fx.floor(), fy.floor());
        let (u, v) = (fx - ix, fy - iy);
        at(ix, iy) * (1.0 - u) * (1.0 - v)
            + at(ix + 1.0, iy) * u * (1.0 - v)
            + at(ix, iy + 1.0) * (1.0 - u) * v
            + at(ix + 1.0, iy + 1.0) * u * v
    };
    bake_heights(reads.key, |lon| lon, |lat| lat, value)
}

// ---------------------------------------------------------------- a snapshot

/// A polygon shapefile of `records` (each its rings of [lat, lon], closed by repeating the first point as a
/// shapefile does), a null shape between the first and the second.
pub fn shapefile(records: &[Vec<Vec<[f64; 2]>>]) -> Vec<u8> {
    let mut out = vec![0u8; 100];
    out[0..4].copy_from_slice(&9994i32.to_be_bytes());
    out[28..32].copy_from_slice(&1000u32.to_le_bytes());
    out[32..36].copy_from_slice(&5u32.to_le_bytes());
    let mut n = 1i32;
    for (k, rings) in records.iter().enumerate() {
        if k == 1 {
            out.extend_from_slice(&n.to_be_bytes());
            out.extend_from_slice(&2i32.to_be_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            n += 1;
        }
        let pts: Vec<[f64; 2]> = rings
            .iter()
            .flat_map(|r| r.iter().chain(std::iter::once(&r[0])).copied())
            .collect();
        let mut c = vec![];
        c.extend_from_slice(&5u32.to_le_bytes());
        let (mut s, mut no, mut w, mut e) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in &pts {
            (s, no, w, e) = (s.min(p[0]), no.max(p[0]), w.min(p[1]), e.max(p[1]));
        }
        for v in [w, s, e, no] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend_from_slice(&(rings.len() as u32).to_le_bytes());
        c.extend_from_slice(&(pts.len() as u32).to_le_bytes());
        let mut start = 0u32;
        for r in rings {
            c.extend_from_slice(&start.to_le_bytes());
            start += r.len() as u32 + 1;
        }
        for p in &pts {
            c.extend_from_slice(&p[1].to_le_bytes());
            c.extend_from_slice(&p[0].to_le_bytes());
        }
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&((c.len() / 2) as i32).to_be_bytes());
        out.extend_from_slice(&c);
        n += 1;
    }
    let words = (out.len() / 2) as i32;
    out[24..28].copy_from_slice(&words.to_be_bytes());
    out
}

/// A zip of `files` (name, bytes), each deflated.
pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = vec![];
    let mut central = vec![];
    for (name, bytes) in files {
        let mut enc = DeflateEncoder::new(vec![], Compression::fast());
        enc.write_all(bytes).unwrap();
        let packed = enc.finish().unwrap();
        let offset = out.len() as u32;
        let mut local = vec![0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0];
        local.extend_from_slice(&[0; 4]);
        local.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        local.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&local);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        let mut c = vec![0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0];
        c.extend_from_slice(&[0; 4]);
        c.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        c.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0; 12]);
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&c);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    let mut end = vec![0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0];
    end.extend_from_slice(&(files.len() as u16).to_le_bytes());
    end.extend_from_slice(&(files.len() as u16).to_le_bytes());
    end.extend_from_slice(&(central.len() as u32).to_le_bytes());
    end.extend_from_slice(&cd_offset.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&end);
    out
}

/// A snapshot zip of `records`, dated 2026-10-14.
pub fn snapshot(records: &[Vec<Vec<[f64; 2]>>]) -> Vec<u8> {
    let readme = b"PACKAGE CONTENT\n\nDate of the data used is 2026-10-14T00:00:00Z\n";
    zip(&[
        ("water-polygons-split-4326/README.txt", readme),
        (
            "water-polygons-split-4326/water_polygons.shp",
            &shapefile(records),
        ),
    ])
}

// ---------------------------------------------------------------- the sea floor's object

/// The GEBCO fixture's bytes before its first block, and the room each block takes after them (its zlib stream, stored
/// uncompressed, then zeros: a reader takes the stream and stops). Its blocks are made when read, so the 86,400 ×
/// 43,200 image costs nothing until a bake asks for a block.
pub const GEBCO_HEADER_LEN: u64 = 262_144;
pub const GEBCO_STRIDE: u64 = 600_000;

/// The fixture's height at GEBCO pixel (column, row): the [`dem`] at its middle, 150 m lower, so it has coasts of its
/// own, banks and deeps.
pub fn gebco_pixel(c: usize, r: usize) -> i16 {
    let per = super::gebco::PER_DEGREE_G as f64;
    let (lat, lon) = (
        90.0 - (r as f64 + 0.5) / per,
        -180.0 + (c as f64 + 0.5) / per,
    );
    (dem(lat, lon) - 150.0).round() as i16
}

/// The fixture object's length.
pub fn gebco_bytes() -> u64 {
    let blocks = super::gebco::WIDTH.div_ceil(512) * super::gebco::HEIGHT.div_ceil(512);
    GEBCO_HEADER_LEN + blocks as u64 * GEBCO_STRIDE
}

/// The fixture's header: a little-endian BigTIFF of GEBCO's layout ([`super::gebco::parse_header`]), every block
/// [`GEBCO_STRIDE`] long one after another.
pub fn gebco_header() -> Vec<u8> {
    use super::gebco::{HEIGHT, PER_DEGREE_G, WIDTH};
    let block = 512usize;
    let n = WIDTH.div_ceil(block) * HEIGHT.div_ceil(block);
    let short = |v: u16| v.to_le_bytes().to_vec();
    let long = |v: u32| v.to_le_bytes().to_vec();
    let doubles = |v: &[f64]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let entries: Vec<(u16, u16, u64, Vec<u8>)> = vec![
        (256, 4, 1, long(WIDTH as u32)),
        (257, 4, 1, long(HEIGHT as u32)),
        (258, 3, 1, short(16)),
        (259, 3, 1, short(8)),
        (262, 3, 1, short(1)),
        (277, 3, 1, short(1)),
        (284, 3, 1, short(1)),
        (317, 3, 1, short(2)),
        (322, 3, 1, short(block as u16)),
        (323, 3, 1, short(block as u16)),
        (
            324,
            16,
            n as u64,
            (0..n as u64)
                .flat_map(|k| (GEBCO_HEADER_LEN + k * GEBCO_STRIDE).to_le_bytes())
                .collect(),
        ),
        (
            325,
            4,
            n as u64,
            (0..n)
                .flat_map(|_| (GEBCO_STRIDE as u32).to_le_bytes())
                .collect(),
        ),
        (339, 3, 1, short(2)),
        (
            33550,
            12,
            3,
            doubles(&[1.0 / PER_DEGREE_G as f64, 1.0 / PER_DEGREE_G as f64, 0.0]),
        ),
        (33922, 12, 6, doubles(&[0.0, 0.0, 0.0, -180.0, 90.0, 0.0])),
    ];
    let ifd = 16u64;
    let mut out = vec![0x49, 0x49, 0x2b, 0x00, 8, 0, 0, 0];
    out.extend_from_slice(&ifd.to_le_bytes());
    let table_end = ifd + 8 + 20 * entries.len() as u64 + 8;
    let mut arrays: Vec<u8> = vec![];
    let mut ifd_bytes = (entries.len() as u64).to_le_bytes().to_vec();
    for (tag, kind, count, data) in &entries {
        ifd_bytes.extend_from_slice(&tag.to_le_bytes());
        ifd_bytes.extend_from_slice(&kind.to_le_bytes());
        ifd_bytes.extend_from_slice(&count.to_le_bytes());
        if data.len() <= 8 {
            let mut v = data.clone();
            v.resize(8, 0);
            ifd_bytes.extend_from_slice(&v);
        } else {
            let at = table_end + arrays.len() as u64;
            ifd_bytes.extend_from_slice(&at.to_le_bytes());
            arrays.extend_from_slice(data);
        }
    }
    ifd_bytes.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&ifd_bytes);
    out.extend_from_slice(&arrays);
    assert!(
        out.len() as u64 <= GEBCO_HEADER_LEN,
        "the fixture's header fits"
    );
    out.resize(GEBCO_HEADER_LEN as usize, 0);
    out
}

/// Block `k`'s bytes as the object holds them: its heights with the horizontal predictor, a stored zlib stream, then
/// zeros to [`GEBCO_STRIDE`].
pub fn gebco_block(k: usize) -> Vec<u8> {
    let block = 512usize;
    let across = super::gebco::WIDTH.div_ceil(block);
    let (br, bc) = (k / across, k % across);
    let mut raw = Vec::with_capacity(2 * block * block);
    for r in 0..block {
        let mut previous = 0i16;
        for c in 0..block {
            let (rr, cc) = (br * block + r, bc * block + c);
            let v = if rr < super::gebco::HEIGHT && cc < super::gebco::WIDTH {
                gebco_pixel(cc, rr)
            } else {
                0
            };
            raw.extend_from_slice(&v.wrapping_sub(previous).to_le_bytes());
            previous = v;
        }
    }
    let mut z = ZlibEncoder::new(vec![], Compression::none());
    z.write_all(&raw).expect("deflate");
    let mut out = z.finish().expect("deflate");
    assert!(
        out.len() as u64 <= GEBCO_STRIDE,
        "a stored block fits its room"
    );
    out.resize(GEBCO_STRIDE as usize, 0);
    out
}

/// Bytes `offset..offset + length` of the fixture object: within its header, or within one block.
pub fn gebco_range(offset: u64, length: u64) -> Vec<u8> {
    if offset + length <= GEBCO_HEADER_LEN {
        return gebco_header()[offset as usize..(offset + length) as usize].to_vec();
    }
    let k = (offset - GEBCO_HEADER_LEN) / GEBCO_STRIDE;
    let inner = offset - GEBCO_HEADER_LEN - k * GEBCO_STRIDE;
    assert!(inner + length <= GEBCO_STRIDE, "a read within one block");
    gebco_block(k as usize)[inner as usize..(inner + length) as usize].to_vec()
}
