//! The snapshot's reader: OpenStreetMap's water polygons as osmdata.openstreetmap.de packs them, a zip holding one
//! polygon shapefile (`water_polygons.shp`, WGS84 longitude and latitude) and a README naming the data's date. The
//! polygons are the sea split into a 1° grid with a slight overlap: each piece is its tile's water, the tile grown by
//! 0.0005° of latitude north and south and 0.0005°/cos φ of longitude east and west (φ the tile's middle latitude), so
//! two neighbours' pieces overlap in a band about 55 m wide. Their union is the sea.
//!
//! Read straight from the zip, no unpacking: the central directory finds the entries, the shapefile is inflated as it
//! is read (a pass over the 1.3 GB of polygons takes seconds). Only what osmdata writes is read: a classic zip (no
//! zip64), entries stored or deflated, polygons (shape type 5) and null shapes.
use std::io::{BufReader, Read, Seek, SeekFrom};

use flate2::read::DeflateDecoder;

use crate::BakeError;

/// A water polygon as the snapshot holds it: its rings, each `[latitude, longitude]` in degrees (closed: the last point
/// is not repeated), and its bounds, south, north, west, east.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterRecord {
    /// Its place in the shapefile, from 0.
    pub index: usize,
    pub rings: Vec<Vec<[f64; 2]>>,
    pub bounds: [f64; 4],
}

fn bad(why: impl std::fmt::Display) -> BakeError {
    BakeError::Corrupt(format!("the water-polygon snapshot: {why}"))
}

fn io(e: std::io::Error) -> BakeError {
    bad(e)
}

/// One entry of the zip's central directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub compressed: u64,
    pub size: u64,
    /// Its local header's offset.
    pub offset: u64,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The zip's entries, from its central directory.
pub fn entries<R: Read + Seek>(zip: &mut R) -> Result<Vec<Entry>, BakeError> {
    let length = zip.seek(SeekFrom::End(0)).map_err(io)?;
    let tail = length.min(65_536 + 22);
    zip.seek(SeekFrom::Start(length - tail)).map_err(io)?;
    let mut end = vec![0u8; tail as usize];
    zip.read_exact(&mut end).map_err(io)?;
    let at = (0..end.len().saturating_sub(21))
        .rev()
        .find(|&k| end[k..k + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| bad("no end of central directory: not a zip"))?;
    let (count, cd_size, cd_offset) = (
        u16_at(&end, at + 10) as usize,
        u32_at(&end, at + 12) as u64,
        u32_at(&end, at + 16) as u64,
    );
    if count == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff {
        return Err(bad("a zip64 archive, which this reader does not read"));
    }
    zip.seek(SeekFrom::Start(cd_offset)).map_err(io)?;
    let mut cd = vec![0u8; cd_size as usize];
    zip.read_exact(&mut cd).map_err(io)?;
    let mut out = Vec::with_capacity(count);
    let mut p = 0;
    for _ in 0..count {
        if p + 46 > cd.len() || cd[p..p + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(bad("a damaged central directory"));
        }
        let (n, e, c) = (
            u16_at(&cd, p + 28) as usize,
            u16_at(&cd, p + 30) as usize,
            u16_at(&cd, p + 32) as usize,
        );
        if p + 46 + n > cd.len() {
            return Err(bad("a damaged central directory"));
        }
        out.push(Entry {
            name: String::from_utf8_lossy(&cd[p + 46..p + 46 + n]).into_owned(),
            method: u16_at(&cd, p + 10),
            compressed: u32_at(&cd, p + 20) as u64,
            size: u32_at(&cd, p + 24) as u64,
            offset: u32_at(&cd, p + 42) as u64,
        });
        p += 46 + n + e + c;
    }
    Ok(out)
}

/// An entry's bytes as they inflate.
pub fn open<'a, R: Read + Seek + 'a>(
    zip: &'a mut R,
    entry: &Entry,
) -> Result<Box<dyn Read + 'a>, BakeError> {
    zip.seek(SeekFrom::Start(entry.offset)).map_err(io)?;
    let mut local = [0u8; 30];
    zip.read_exact(&mut local).map_err(io)?;
    if local[..4] != [0x50, 0x4b, 0x03, 0x04] {
        return Err(bad(format!("{}: no local header", entry.name)));
    }
    let skip = u16_at(&local, 26) as i64 + u16_at(&local, 28) as i64;
    zip.seek(SeekFrom::Current(skip)).map_err(io)?;
    let raw = zip.take(entry.compressed);
    match entry.method {
        0 => Ok(Box::new(raw)),
        8 => Ok(Box::new(DeflateDecoder::new(raw))),
        m => Err(bad(format!("{}: compression method {m}", entry.name))),
    }
}

fn find<'e>(entries: &'e [Entry], suffix: &str) -> Result<&'e Entry, BakeError> {
    let mut found = entries.iter().filter(|e| e.name.ends_with(suffix));
    match (found.next(), found.next()) {
        (Some(e), None) => Ok(e),
        (None, _) => Err(bad(format!("no {suffix} in the zip"))),
        _ => Err(bad(format!("more than one {suffix} in the zip"))),
    }
}

/// The data's date as the snapshot's README states it (`Date of the data used is 2026-10-14T00:00:00Z`): `2026-10-14`.
pub fn date<R: Read + Seek>(zip: &mut R) -> Result<String, BakeError> {
    let all = entries(zip)?;
    let readme = find(&all, "README.txt")?.clone();
    let mut text = String::new();
    open(zip, &readme)?
        .take(1 << 20)
        .read_to_string(&mut text)
        .map_err(io)?;
    let marker = "Date of the data used is ";
    let at = text
        .find(marker)
        .ok_or_else(|| bad("the README states no date"))?;
    let date: String = text[at + marker.len()..].chars().take(10).collect();
    let ok = date.len() == 10
        && date.bytes().enumerate().all(|(k, b)| {
            if k == 4 || k == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !ok {
        return Err(bad(format!("the README's date {date:?} is not a date")));
    }
    Ok(date)
}

/// Every polygon of the snapshot's shapefile, in order, given to `each` one at a time (the file is read once, never
/// held whole). Stops at the first error `each` returns.
pub fn for_each<R: Read + Seek>(
    zip: &mut R,
    mut each: impl FnMut(WaterRecord) -> Result<(), BakeError>,
) -> Result<usize, BakeError> {
    let all = entries(zip)?;
    let shp = find(&all, ".shp")?.clone();
    let mut reader = BufReader::with_capacity(1 << 20, open(zip, &shp)?);
    read_shapefile(&mut reader, shp.size, &mut each)
}

/// A polygon shapefile's records (its `.shp` bytes as they are read), `length` bytes in all.
pub fn read_shapefile(
    reader: &mut dyn Read,
    length: u64,
    each: &mut dyn FnMut(WaterRecord) -> Result<(), BakeError>,
) -> Result<usize, BakeError> {
    let mut header = [0u8; 100];
    reader.read_exact(&mut header).map_err(io)?;
    let code = i32::from_be_bytes(header[0..4].try_into().unwrap());
    let words = i32::from_be_bytes(header[24..28].try_into().unwrap()) as u64;
    let kind = u32_at(&header, 32);
    if code != 9994 || words * 2 != length || kind != 5 {
        return Err(bad(format!(
            "not a polygon shapefile of its length (code {code}, {} bytes of {length}, shape type {kind})",
            words * 2
        )));
    }
    let mut at = 100u64;
    let mut index = 0usize;
    let mut body = Vec::new();
    while at < length {
        let mut head = [0u8; 8];
        reader.read_exact(&mut head).map_err(io)?;
        let content = i32::from_be_bytes(head[4..8].try_into().unwrap());
        if content < 2 || at + 8 + 2 * content as u64 > length {
            return Err(bad(format!("record {index} runs past the file")));
        }
        body.resize(2 * content as usize, 0);
        reader.read_exact(&mut body).map_err(io)?;
        at += 8 + body.len() as u64;
        if let Some(record) = polygon(&body, index)? {
            each(record)?;
        }
        index += 1;
    }
    Ok(index)
}

/// A polygon record's content (none for a null shape).
fn polygon(b: &[u8], index: usize) -> Result<Option<WaterRecord>, BakeError> {
    let kind = u32_at(b, 0);
    if kind == 0 {
        return Ok(None);
    }
    let f64_at = |at: usize| f64::from_le_bytes(b[at..at + 8].try_into().unwrap());
    if kind != 5 || b.len() < 44 {
        return Err(bad(format!(
            "record {index} is shape type {kind}, not a polygon"
        )));
    }
    let (parts, points) = (u32_at(b, 36) as usize, u32_at(b, 40) as usize);
    if b.len() != 44 + 4 * parts + 16 * points || parts == 0 {
        return Err(bad(format!(
            "record {index}'s length is not its parts' and points'"
        )));
    }
    let starts: Vec<usize> = (0..parts).map(|k| u32_at(b, 44 + 4 * k) as usize).collect();
    let base = 44 + 4 * parts;
    let mut rings = Vec::with_capacity(parts);
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(points);
        if start >= end || end > points {
            return Err(bad(format!("record {index}'s parts are out of order")));
        }
        let mut ring: Vec<[f64; 2]> = (start..end)
            .map(|p| [f64_at(base + 16 * p + 8), f64_at(base + 16 * p)])
            .collect();
        if ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    let (west, south, east, north) = (f64_at(4), f64_at(12), f64_at(20), f64_at(28));
    let finite = [west, south, east, north].iter().all(|v| v.is_finite());
    if !finite || south > north || west > east || east - west >= 180.0 || rings.is_empty() {
        return Err(bad(format!("record {index}'s bounds are not a polygon's")));
    }
    Ok(Some(WaterRecord {
        index,
        rings,
        bounds: [south, north, west, east],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use super::super::fixtures::{shapefile, snapshot, zip};

    #[test]
    fn a_snapshot_reads_its_date_and_its_polygons_in_order() {
        let a = vec![
            vec![[10.0, 20.0], [10.0, 21.0], [11.0, 21.0], [11.0, 20.0]],
            vec![[10.4, 20.4], [10.6, 20.4], [10.6, 20.6]],
        ];
        let b = vec![vec![[-5.0, 179.5], [-5.0, 180.0], [-4.0, 180.0]]];
        let bytes = snapshot(&[a.clone(), b.clone()]);
        let mut file = Cursor::new(bytes);
        assert_eq!(date(&mut file).unwrap(), "2026-10-14");
        let mut got = vec![];
        let n = for_each(&mut file, |r| {
            got.push(r);
            Ok(())
        })
        .unwrap();
        assert_eq!(n, 3, "two polygons and a null shape");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].rings, a);
        assert_eq!(got[0].bounds, [10.0, 11.0, 20.0, 21.0]);
        assert_eq!((got[1].index, &got[1].rings), (2, &b));
        // A truncated shapefile, a zip with none.
        let mut short = shapefile(&[a]);
        short.truncate(short.len() - 9);
        let mut cut = Cursor::new(zip_of_shp(&short));
        assert!(for_each(&mut cut, |_| Ok(())).is_err());
        let mut none = Cursor::new(zip(&[("README.txt", b"x")]));
        assert!(for_each(&mut none, |_| Ok(())).is_err());
    }

    fn zip_of_shp(shp: &[u8]) -> Vec<u8> {
        zip(&[("w/water_polygons.shp", shp)])
    }
}
