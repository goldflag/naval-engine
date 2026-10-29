use std::collections::HashMap;
use std::sync::Mutex;

use parquet::basic::{Compression, ZstdLevel};
use parquet::column::writer::ColumnWriter;
use parquet::data_type::ByteArray;
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;

use super::*;

/// The bucket in memory: each key's bytes, and the requests made.
struct Bucket {
    files: HashMap<String, Vec<u8>>,
    requests: Mutex<Vec<String>>,
}

impl Fetch for Bucket {
    fn get(&self, url: &str, range: Option<std::ops::Range<u64>>) -> Result<Vec<u8>, String> {
        self.requests
            .lock()
            .unwrap()
            .push(format!("{url} {range:?}"));
        let key = url
            .strip_prefix(&format!("{BUCKET_URL}/"))
            .ok_or("not the bucket")?;
        let bytes = self.files.get(key).ok_or("no such key")?;
        Ok(match range {
            Some(r) => bytes[r.start as usize..r.end as usize].to_vec(),
            None => bytes.clone(),
        })
    }
}

/// One row: its subtype, class, height and geometry.
type Row = (&'static str, &'static str, Option<f64>, Vec<u8>);

fn bounds(geometry: &[u8]) -> [f64; 4] {
    let points = match wkb::decode(geometry).unwrap() {
        wkb::Shape::Point(p) => vec![p],
        wkb::Shape::Lines(l) => l.concat(),
    };
    points
        .iter()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        })
}

/// A GeoParquet file as Overture's infrastructure (the columns read and a few others), a row group per `groups`.
fn write(groups: &[Vec<Row>]) -> Vec<u8> {
    let schema = Arc::new(
        parse_message_type(
            "message schema {
                REQUIRED BYTE_ARRAY id (UTF8);
                OPTIONAL BYTE_ARRAY geometry;
                OPTIONAL BYTE_ARRAY subtype (UTF8);
                OPTIONAL BYTE_ARRAY class (UTF8);
                OPTIONAL DOUBLE height;
                OPTIONAL group bbox {
                    OPTIONAL DOUBLE xmin;
                    OPTIONAL DOUBLE xmax;
                    OPTIONAL DOUBLE ymin;
                    OPTIONAL DOUBLE ymax;
                }
            }",
        )
        .unwrap(),
    );
    let props = Arc::new(
        WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            .build(),
    );
    let mut out = Vec::new();
    let mut writer = SerializedFileWriter::new(&mut out, schema, props).unwrap();
    for rows in groups {
        let mut group = writer.next_row_group().unwrap();
        let mut column = 0;
        while let Some(mut col) = group.next_column().unwrap() {
            let ones = vec![1i16; rows.len()];
            let twos = vec![2i16; rows.len()];
            match (column, col.untyped()) {
                (0, ColumnWriter::ByteArrayColumnWriter(w)) => {
                    let ids: Vec<ByteArray> = (0..rows.len())
                        .map(|i| format!("id{i}").as_str().into())
                        .collect();
                    w.write_batch(&ids, None, None).unwrap();
                }
                (1..=3, ColumnWriter::ByteArrayColumnWriter(w)) => {
                    let values: Vec<ByteArray> = rows
                        .iter()
                        .map(|r| match column {
                            1 => ByteArray::from(r.3.clone()),
                            2 => r.0.into(),
                            _ => r.1.into(),
                        })
                        .collect();
                    w.write_batch(&values, Some(&ones), None).unwrap();
                }
                (4, ColumnWriter::DoubleColumnWriter(w)) => {
                    let values: Vec<f64> = rows.iter().filter_map(|r| r.2).collect();
                    let levels: Vec<i16> = rows.iter().map(|r| i16::from(r.2.is_some())).collect();
                    w.write_batch(&values, Some(&levels), None).unwrap();
                }
                (5..=8, ColumnWriter::DoubleColumnWriter(w)) => {
                    let pick = [0, 2, 1, 3][column - 5];
                    let values: Vec<f64> = rows.iter().map(|r| bounds(&r.3)[pick]).collect();
                    w.write_batch(&values, Some(&twos), None).unwrap();
                }
                (c, _) => panic!("column {c}"),
            }
            col.close().unwrap();
            column += 1;
        }
        group.close().unwrap();
    }
    writer.close().unwrap();
    out
}

fn bucket() -> Bucket {
    let la = vec![
        (
            "power",
            "power_line",
            None,
            wkb::line_string(&[[-118.40, 33.90], [-118.39, 33.91], [-118.38, 33.91]]),
        ),
        (
            "power",
            "power_tower",
            Some(42.0),
            wkb::point([-118.39, 33.91]),
        ),
        ("power", "power_pole", None, wkb::point([-118.41, 33.92])),
        (
            "emergency",
            "fire_hydrant",
            None,
            wkb::point([-118.40, 33.93]),
        ),
        (
            "communication",
            "communication_tower",
            Some(120.0),
            wkb::point([-118.42, 33.95]),
        ),
        (
            "power",
            "minor_line",
            None,
            wkb::line_string(&[[-118.0, 34.5], [-117.9, 34.6]]),
        ),
    ];
    let europe = vec![(
        "power",
        "power_line",
        None,
        wkb::line_string(&[[10.0, 50.0], [10.1, 50.1]]),
    )];
    Bucket {
        files: HashMap::from([("rel/a.parquet".to_string(), write(&[la, europe]))]),
        requests: Mutex::new(Vec::new()),
    }
}

#[test]
fn a_box_reads_only_its_row_groups_and_keeps_the_power() {
    let bucket = bucket();
    let size = bucket.files["rel/a.parquet"].len() as u64;
    let dir = std::env::temp_dir().join(format!("naval-overture-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = vec![DataFile {
        key: "rel/a.parquet".into(),
        size,
    }];
    let release = Release::with_files(&bucket, &dir, "test", files.clone()).unwrap();
    assert_eq!(release.row_groups(), 2);
    // The footer: its length, then itself.
    assert_eq!(bucket.requests.lock().unwrap().len(), 2);
    let read = release
        .read_box(&bucket, [-118.5, 33.8], [-118.3, 34.0])
        .unwrap();
    // One row group, its four columns.
    assert_eq!(read.row_groups, 1);
    assert_eq!(bucket.requests.lock().unwrap().len(), 6);
    let cell = &read.cell;
    assert_eq!(cell.lines.len(), 1, "{cell:?}");
    assert_eq!(cell.lines[0].class, LineClass::Power);
    assert_eq!(cell.lines[0].points.len(), 3);
    let mut kinds: Vec<(MastKind, f32)> = cell.masts.iter().map(|m| (m.kind, m.height)).collect();
    kinds.sort_by_key(|k| k.0 as u8);
    assert_eq!(
        kinds,
        [
            (MastKind::PowerTower, 42.0),
            (MastKind::PowerPole, 0.0),
            (MastKind::Communication, 120.0)
        ]
    );
    // Kept on disk: opened again, no request.
    let before = bucket.requests.lock().unwrap().len();
    let again = Release::with_files(&bucket, &dir, "test", files).unwrap();
    assert_eq!(bucket.requests.lock().unwrap().len(), before);
    assert_eq!(
        again
            .read_box(&bucket, [-118.5, 33.8], [-118.3, 34.0])
            .unwrap(),
        read
    );
    // The cell kept as bytes.
    assert_eq!(PowerCell::decode(&cell.encode()).as_ref(), Some(cell));
    assert_eq!(PowerCell::decode(&cell.encode()[..30]), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_newest_release_and_its_files_come_from_the_listings() {
    let xml = "<ListBucketResult><CommonPrefixes><Prefix>release/2026-08-19.0/</Prefix></CommonPrefixes>\
        <CommonPrefixes><Prefix>release/2026-09-23.1/</Prefix></CommonPrefixes>\
        <CommonPrefixes><Prefix>release/2026-09-23.0/</Prefix></CommonPrefixes></ListBucketResult>";
    assert_eq!(newest_release(xml).as_deref(), Some("2026-09-23.1"));
    let listing = "<ListBucketResult><IsTruncated>false</IsTruncated>\
        <Contents><Key>release/r/theme=base/type=infrastructure/part-0.zstd.parquet</Key><Size>221558044</Size></Contents>\
        <Contents><Key>release/r/theme=base/type=infrastructure/_SUCCESS</Key><Size>0</Size></Contents>\
        </ListBucketResult>";
    let bucket = Bucket {
        files: HashMap::from([(
            "?list-type=2&prefix=release/r/theme=base/type=infrastructure/".to_string(),
            listing.as_bytes().to_vec(),
        )]),
        requests: Mutex::new(Vec::new()),
    };
    assert_eq!(
        infrastructure_files(&bucket, "r").unwrap(),
        [DataFile {
            key: "release/r/theme=base/type=infrastructure/part-0.zstd.parquet".into(),
            size: 221_558_044
        }]
    );
}
