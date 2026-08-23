//! Regenerate `fixtures/`. Run with `pnpm --filter @voxelkloud/wasm-codecs fixtures`.
//!
//! For each point format it writes four files:
//!
//! - `fmtN.las`   an uncompressed LAS file. Its point block is the oracle.
//! - `fmtN.laz`   the same points as a LAZ file, in several fixed-size chunks,
//!                so the whole-file path meets a real chunk table.
//! - `fmtN.vlr`   a laszip VLR payload in variable-size-chunk mode.
//! - `fmtN.chunk` all the points as one bare chunk under that VLR — no chunk
//!                table, no leading offset. This is the shape COPC stores per
//!                node, and the only way to test that path without a COPC file.
//!
//! The points come out of the same library that decodes them, so these fixtures
//! prove the wasm boundary, the framing and the chunk path — not laszip
//! conformance, which is laz-rs's own test suite's job. Conformance is covered
//! by `real-files.test.ts`, against files this repo did not produce.

use std::fs;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::Path;

use laz::{LasZipCompressor, LazItemRecordBuilder, LazVlrBuilder};

/// Enough points for several chunks without making the repo heavier than the
/// fixtures are worth.
const POINT_COUNT: u32 = 512;
/// Small on purpose: 512 points land in four chunks.
const CHUNK_SIZE: u32 = 128;

const SCALE: f64 = 0.001;

struct Spec {
    format: u8,
    extra: u16,
}

const SPECS: &[Spec] = &[
    Spec { format: 0, extra: 0 },
    Spec { format: 2, extra: 0 },
    Spec { format: 3, extra: 0 },
    // Extra bytes with no descriptor VLR: the codec never interprets them, and
    // a driver that does is reading the descriptor itself.
    Spec { format: 3, extra: 4 },
    Spec { format: 6, extra: 0 },
    Spec { format: 7, extra: 0 },
    Spec { format: 8, extra: 0 },
];

fn base_point_size(format: u8) -> u16 {
    match format {
        0 => 20,
        1 => 28,
        2 => 26,
        3 => 34,
        6 => 30,
        7 => 36,
        8 => 38,
        other => panic!("fixture generator does not know point format {other}"),
    }
}

/// LAS 1.4 is required for point formats 6 and up.
fn version_for(format: u8) -> (u8, u8) {
    if format >= 6 {
        (1, 4)
    } else {
        (1, 2)
    }
}

fn header_size(version: (u8, u8)) -> u16 {
    match version {
        (1, 4) => 375,
        _ => 227,
    }
}

struct Field(Vec<u8>);

impl Field {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn i8(&mut self, v: i8) -> &mut Self {
        self.0.push(v as u8);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
}

/// Coordinates for point `i`, in scaled integers.
///
/// A smooth ramp with a small wobble rather than noise: laszip predicts from
/// the previous point, so random coordinates would compress to nothing and
/// exercise none of the arithmetic that matters.
fn xyz(i: u32) -> (i32, i32, i32) {
    let i = i as i32;
    (
        100_000 + i * 37 + (i % 7) * 3,
        200_000 + i * 53 - (i % 11) * 5,
        5_000 + (i % 97) * 11,
    )
}

fn point_records(spec: &Spec) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        (base_point_size(spec.format) + spec.extra) as usize * POINT_COUNT as usize,
    );

    for i in 0..POINT_COUNT {
        let (x, y, z) = xyz(i);
        let mut f = Field::new();
        f.i32(x).i32(y).i32(z);
        f.u16((i * 7 % 65536) as u16); // intensity

        if spec.format < 6 {
            let return_number = (i % 5) + 1;
            let number_of_returns = 5u32;
            let bits = (return_number & 0b111)
                | ((number_of_returns & 0b111) << 3)
                | ((i % 2) << 6) // scan direction
                | ((i % 13 == 0) as u32) << 7; // edge of flight line
            f.u8(bits as u8);
            // Kept under 32 so the synthetic/keypoint/withheld bits stay clear.
            f.u8((i % 32) as u8);
            f.i8(((i % 181) as i32 - 90) as i8); // scan angle rank
            f.u8((i % 256) as u8); // user data
            f.u16((i % 1000) as u16); // point source id
        } else {
            let return_number = (i % 5) + 1;
            let number_of_returns = 5u32;
            f.u8(((return_number & 0xf) | ((number_of_returns & 0xf) << 4)) as u8);
            let class_flags = (i % 16) & 0b1111;
            let scanner_channel = i % 4;
            let bits2 = class_flags | (scanner_channel << 4) | ((i % 2) << 6);
            f.u8(bits2 as u8);
            f.u8((i % 256) as u8); // classification, full byte in 1.4
            f.u8((i % 256) as u8); // user data
            f.i16(((i as i32 % 60_001) - 30_000) as i16); // scan angle, 0.006 deg
            f.u16((i % 1000) as u16); // point source id
        }

        let has_gps = matches!(spec.format, 1 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10);
        if has_gps {
            f.f64(1_000.0 + i as f64 * 0.001);
        }
        if matches!(spec.format, 2 | 3 | 5 | 7 | 8 | 10) {
            f.u16((i * 13 % 65536) as u16)
                .u16((i * 29 % 65536) as u16)
                .u16((i * 41 % 65536) as u16);
        }
        if matches!(spec.format, 8 | 10) {
            f.u16((i * 17 % 65536) as u16);
        }
        for b in 0..spec.extra {
            f.u8(((i + b as u32) % 251) as u8);
        }

        let expect = (base_point_size(spec.format) + spec.extra) as usize;
        assert_eq!(
            f.0.len(),
            expect,
            "point format {} built {} bytes, expected {expect}",
            spec.format,
            f.0.len()
        );
        out.extend_from_slice(&f.0);
    }

    out
}

fn fixed_bytes(text: &str, len: usize) -> Vec<u8> {
    let mut v = text.as_bytes().to_vec();
    v.resize(len, 0);
    v
}

/// The LAS public header block, followed by the laszip VLR when there is one.
fn las_header(spec: &Spec, laszip_record: Option<&[u8]>) -> Vec<u8> {
    let version = version_for(spec.format);
    let hsize = header_size(version);
    let point_size = base_point_size(spec.format) + spec.extra;
    let vlr_bytes = laszip_record.map_or(0, |r| 54 + r.len());

    let (mut min, mut max) = ([i32::MAX; 3], [i32::MIN; 3]);
    for i in 0..POINT_COUNT {
        let (x, y, z) = xyz(i);
        for (axis, v) in [x, y, z].into_iter().enumerate() {
            min[axis] = min[axis].min(v);
            max[axis] = max[axis].max(v);
        }
    }

    let mut h = vec![0u8; hsize as usize];
    let mut w = Cursor::new(&mut h);
    let put = |w: &mut Cursor<&mut Vec<u8>>, at: u64, bytes: &[u8]| {
        w.seek(SeekFrom::Start(at)).unwrap();
        w.write_all(bytes).unwrap();
    };

    put(&mut w, 0, b"LASF");
    put(&mut w, 6, &1u16.to_le_bytes()); // global encoding: adjusted standard GPS time
    put(&mut w, 24, &[version.0, version.1]);
    put(&mut w, 26, &fixed_bytes("voxelkloud fixtures", 32));
    put(&mut w, 58, &fixed_bytes("voxelkloud-wasm-codecs", 32));
    put(&mut w, 90, &1u16.to_le_bytes()); // creation day
    put(&mut w, 92, &2026u16.to_le_bytes()); // creation year
    put(&mut w, 94, &hsize.to_le_bytes());
    put(&mut w, 96, &(hsize as u32 + vlr_bytes as u32).to_le_bytes());
    put(&mut w, 100, &(laszip_record.is_some() as u32).to_le_bytes());
    let format_byte = spec.format | if laszip_record.is_some() { 0x80 } else { 0 };
    put(&mut w, 104, &[format_byte]);
    put(&mut w, 105, &point_size.to_le_bytes());

    // LAS 1.4 requires the legacy counts to be zero for point formats 6 and up,
    // which is also the case the header reader has to get right: it must read
    // the 64-bit field and not the zero in front of it.
    let legacy_count = if version == (1, 4) { 0 } else { POINT_COUNT };
    put(&mut w, 107, &legacy_count.to_le_bytes());

    for (at, v) in [(131, SCALE), (139, SCALE), (147, SCALE)] {
        put(&mut w, at, &v.to_le_bytes());
    }
    for at in [155u64, 163, 171] {
        put(&mut w, at, &0f64.to_le_bytes()); // offsets
    }
    // The header interleaves the bounds as max then min, per axis.
    for (axis, (max_at, min_at)) in [(179u64, 187u64), (195, 203), (211, 219)]
        .into_iter()
        .enumerate()
    {
        put(&mut w, max_at, &(max[axis] as f64 * SCALE).to_le_bytes());
        put(&mut w, min_at, &(min[axis] as f64 * SCALE).to_le_bytes());
    }

    if version == (1, 4) {
        put(&mut w, 247, &(POINT_COUNT as u64).to_le_bytes());
    }

    if let Some(record) = laszip_record {
        h.extend_from_slice(&0u16.to_le_bytes()); // reserved
        h.extend_from_slice(&fixed_bytes("laszip encoded", 16));
        h.extend_from_slice(&22204u16.to_le_bytes());
        h.extend_from_slice(&(record.len() as u16).to_le_bytes());
        h.extend_from_slice(&fixed_bytes("laz-rs", 32));
        h.extend_from_slice(record);
    }

    h
}

fn laz_items(spec: &Spec) -> Vec<laz::LazItem> {
    LazItemRecordBuilder::default_for_point_format_id(spec.format, spec.extra)
        .expect("point format the fixture list claims laz-rs supports")
}

/// Compress every point into one chunk and return just that chunk's bytes.
///
/// The compressor writes an 8-byte chunk-table offset, then the chunks, then
/// the table. Slicing between the two leaves exactly what a COPC node holds.
fn bare_chunk(spec: &Spec, points: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let vlr = LazVlrBuilder::new(laz_items(spec))
        .with_variable_chunk_size()
        .build();

    let mut record = Vec::new();
    vlr.write_to(&mut record).unwrap();

    let mut compressor = LasZipCompressor::new(Cursor::new(Vec::new()), vlr).unwrap();
    compressor.compress_many(points).unwrap();
    compressor.done().unwrap();

    let bytes = compressor.into_inner().into_inner();
    let table_at = i64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    assert!(
        table_at > 8 && table_at <= bytes.len(),
        "chunk table offset {table_at} is outside the {} bytes written",
        bytes.len()
    );
    (record, bytes[8..table_at].to_vec())
}

/// A whole LAZ file, chunked, chunk table and all.
fn laz_file(spec: &Spec, points: &[u8]) -> Vec<u8> {
    let vlr = LazVlrBuilder::new(laz_items(spec))
        .with_fixed_chunk_size(CHUNK_SIZE)
        .build();

    let mut record = Vec::new();
    vlr.write_to(&mut record).unwrap();

    // Compress into the file itself rather than into a scratch buffer: the
    // chunk-table offset laszip writes is absolute in the stream it was handed,
    // so a body built on its own is off by the length of the header in front of
    // it — and every reader then seeks into the middle of the point data.
    let mut cursor = Cursor::new(las_header(spec, Some(&record)));
    cursor.seek(SeekFrom::End(0)).unwrap();

    let mut compressor = LasZipCompressor::new(cursor, vlr).unwrap();
    compressor.compress_many(points).unwrap();
    compressor.done().unwrap();
    compressor.into_inner().into_inner()
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    fs::create_dir_all(&dir).unwrap();

    for spec in SPECS {
        let name = if spec.extra == 0 {
            format!("fmt{}", spec.format)
        } else {
            format!("fmt{}-extra{}", spec.format, spec.extra)
        };
        let points = point_records(spec);

        let mut las = las_header(spec, None);
        las.extend_from_slice(&points);
        fs::write(dir.join(format!("{name}.las")), &las).unwrap();

        fs::write(dir.join(format!("{name}.laz")), laz_file(spec, &points)).unwrap();

        let (record, chunk) = bare_chunk(spec, &points);
        fs::write(dir.join(format!("{name}.vlr")), &record).unwrap();
        fs::write(dir.join(format!("{name}.chunk")), &chunk).unwrap();

        println!(
            "{name}: {POINT_COUNT} points, {} bytes raw, {} bytes as one chunk",
            points.len(),
            chunk.len()
        );
    }
}
